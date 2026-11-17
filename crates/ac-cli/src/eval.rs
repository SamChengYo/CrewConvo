//! `ac-cli decider eval`: precision / recall / mode accuracy / latency per Decider config,
//! plus threshold sweeps for T_WAKE and T_DISCUSS (spec 6.2).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ac_core::model::{
    AgentOrigin, AgentProfile, AgentStatus, DeciderConfig, DeciderProtocol, Interaction, MemberKind, WakeMode,
};
use ac_decider::secret::last4;
use ac_decider::{
    DecideConversation, DecideInput, DecideMessage, DeciderChain, Decision, EgressPolicy, SecretBox, resolve,
};
use ac_store::{DeciderConfigRecord, Store};
use chrono::Utc;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub struct EvalArgs {
    pub fixtures: PathBuf,
    pub config: Option<Uuid>,
    pub workspace: Option<Uuid>,
    pub env_configs: bool,
    pub out: Option<PathBuf>,
    pub raw: Option<PathBuf>,
    pub concurrency: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct FxMessage {
    author: String,
    #[serde(default)]
    kind: Option<String>,
    body: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FxCandidate {
    handle: String,
    #[serde(default)]
    display_name: Option<String>,
    description: String,
    #[serde(default)]
    expertise: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FxConversation {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    topic: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Fixture {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    conversation: Option<FxConversation>,
    #[serde(default)]
    recent: Vec<FxMessage>,
    message: FxMessage,
    candidates: Vec<FxCandidate>,
    expected_wake: Vec<String>,
    expected_mode: String,
}

#[derive(Debug, Clone, Serialize)]
struct Outcome {
    fixture: String,
    category: String,
    ok: bool,
    error: Option<String>,
    latency_ms: u64,
    scores: Vec<(String, f32)>,
    interaction: Option<String>,
    interaction_confidence: f32,
    expected_wake: Vec<String>,
    expected_mode: String,
}

fn env_records(secrets: &SecretBox) -> Vec<DeciderConfigRecord> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let mk = |name: &str,
              protocol: DeciderProtocol,
              url: &str,
              header: &str,
              prefix: &str,
              model: &str,
              key: &str,
              timeout: i32,
              extra: serde_json::Value,
              prio: i32| {
        DeciderConfigRecord {
            config: DeciderConfig {
                id: Uuid::now_v7(),
                workspace_id: Uuid::nil(),
                name: name.into(),
                protocol,
                endpoint_url: url.into(),
                has_api_key: true,
                api_key_last4: Some(last4(key)),
                auth_header: header.into(),
                auth_prefix: prefix.into(),
                model: model.into(),
                input_field: "input".into(),
                timeout_ms: timeout,
                t_wake: 0.6,
                t_discuss: 0.7,
                max_wake: 4,
                extra_headers: json!({}),
                extra_body: extra,
                enabled: true,
                priority: prio,
                updated_by: None,
                updated_at: Utc::now(),
            },
            api_key_encrypted: Some(secrets.encrypt(key)),
        }
    };
    let mut v = Vec::new();
    if let Some(k) = env("GEMINI_API_KEY") {
        v.push(mk(
            "Gemini Decisions API (gemini-decision-1)",
            DeciderProtocol::DecisionsApi,
            "https://generativelanguage.googleapis.com/v1beta/decisions",
            "x-goog-api-key",
            "",
            "gemini-decision-1",
            &k,
            5000,
            json!({}),
            0,
        ));
        v.push(mk(
            "llm_structured · gemini-4-hydrogen",
            DeciderProtocol::LlmStructured,
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            "Authorization",
            "Bearer ",
            "gemini-4-hydrogen",
            &k,
            30000,
            json!({}),
            2,
        ));
    }
    if let Some(k) = env("OPENAI_API_KEY") {
        v.push(mk(
            "llm_structured · gpt-6-luna",
            DeciderProtocol::LlmStructured,
            "https://api.openai.com/v1/chat/completions",
            "Authorization",
            "Bearer ",
            "gpt-6-luna",
            &k,
            30000,
            json!({"reasoning_effort": "none"}),
            1,
        ));
    }
    v
}

fn to_input(f: &Fixture) -> (DecideInput, HashMap<Uuid, String>) {
    let mut ids = HashMap::new();
    let candidates = f
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let id = Uuid::from_u128(i as u128 + 1);
            ids.insert(id, c.handle.clone());
            AgentProfile {
                id,
                handle: c.handle.clone(),
                display_name: c.display_name.clone().unwrap_or_else(|| c.handle.clone()),
                origin: AgentOrigin::Builtin,
                description: c.description.clone(),
                expertise: c.expertise.clone(),
                status: AgentStatus::Online,
            }
        })
        .collect();
    let kind = |k: &Option<String>| if k.as_deref() == Some("agent") { MemberKind::Agent } else { MemberKind::Human };
    let conv = f.conversation.clone().unwrap_or(FxConversation { name: Some("general".into()), topic: None });
    (
        DecideInput {
            message: DecideMessage {
                author_handle: f.message.author.clone(),
                author_kind: MemberKind::Human,
                body: f.message.body.clone(),
            },
            recent: f
                .recent
                .iter()
                .map(|m| DecideMessage {
                    author_handle: m.author.clone(),
                    author_kind: kind(&m.kind),
                    body: m.body.clone(),
                })
                .collect(),
            conversation: DecideConversation { name: conv.name, topic: conv.topic, is_dm: false },
            candidates,
        },
        ids,
    )
}

#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    tp: usize,
    fp: usize,
    fn_: usize,
    exact: usize,
    mode_ok: usize,
    n: usize,
}

impl Metrics {
    fn precision(&self) -> f64 {
        if self.tp + self.fp == 0 { 1.0 } else { self.tp as f64 / (self.tp + self.fp) as f64 }
    }
    fn recall(&self) -> f64 {
        if self.tp + self.fn_ == 0 { 1.0 } else { self.tp as f64 / (self.tp + self.fn_) as f64 }
    }
    fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 { 0.0 } else { 2.0 * p * r / (p + r) }
    }
    fn exact_rate(&self) -> f64 {
        if self.n == 0 { 0.0 } else { self.exact as f64 / self.n as f64 }
    }
    fn mode_rate(&self) -> f64 {
        if self.n == 0 { 0.0 } else { self.mode_ok as f64 / self.n as f64 }
    }
}

fn evaluate(outcomes: &[Outcome], base: &DeciderConfig, t_wake: f32, t_discuss: f32, filter: Option<&str>) -> Metrics {
    let mut m = Metrics::default();
    let mut cfg = base.clone();
    cfg.t_wake = t_wake;
    cfg.t_discuss = t_discuss;
    for o in outcomes.iter().filter(|o| filter.map(|c| o.category == c).unwrap_or(true)) {
        m.n += 1;
        let predicted: Vec<String>;
        let mode: Option<WakeMode>;
        if o.ok {
            let ids: Vec<(Uuid, f32)> =
                o.scores.iter().enumerate().map(|(i, (_, s))| (Uuid::from_u128(i as u128 + 1), *s)).collect();
            let d = Decision {
                scores: ids,
                interaction: if o.interaction.as_deref() == Some("discuss") {
                    Interaction::Discuss
                } else {
                    Interaction::Independent
                },
                interaction_confidence: o.interaction_confidence,
            };
            let (w, md) = resolve(&d, &cfg);
            predicted = w.iter().map(|(id, _)| o.scores[(id.as_u128() - 1) as usize].0.clone()).collect();
            mode = md;
        } else {
            predicted = vec![];
            mode = None;
        }
        for p in &predicted {
            if o.expected_wake.contains(p) {
                m.tp += 1;
            } else {
                m.fp += 1;
            }
        }
        m.fn_ += o.expected_wake.iter().filter(|e| !predicted.contains(e)).count();
        let mut a = predicted.clone();
        a.sort();
        let mut b = o.expected_wake.clone();
        b.sort();
        if a == b {
            m.exact += 1;
        }
        let pm = mode.map(|x| x.as_str().to_string()).unwrap_or_else(|| "none".into());
        if pm == o.expected_mode {
            m.mode_ok += 1;
        }
    }
    m
}

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn percentile(v: &mut [u64], p: f64) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort();
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx]
}

pub async fn run(args: EvalArgs, store: Option<Store>) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(&args.fixtures)?;
    let fixtures: Vec<Fixture> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| anyhow::anyhow!("line {}: {e}", i + 1)))
        .collect::<Result<_, _>>()?;
    println!("loaded {} fixtures", fixtures.len());

    let (secrets, mut records) = if args.env_configs {
        let s = SecretBox::ephemeral();
        let r = env_records(&s);
        (s, r)
    } else {
        let store = store.ok_or_else(|| anyhow::anyhow!("database required"))?;
        let master =
            std::env::var("CREWCONVO_MASTER_KEY").map_err(|_| anyhow::anyhow!("CREWCONVO_MASTER_KEY not set"))?;
        let s = SecretBox::from_base64(&master).map_err(anyhow::Error::msg)?;
        let ws = match args.workspace {
            Some(w) => w,
            None => store.default_workspace().await?.ok_or_else(|| anyhow::anyhow!("no workspace"))?.id,
        };
        (s, store.list_decider_configs(ws).await?.into_iter().filter(|r| r.config.enabled).collect())
    };
    if let Some(id) = args.config {
        records.retain(|r| r.config.id == id);
    }
    anyhow::ensure!(!records.is_empty(), "no decider configs to evaluate");
    let chain = Arc::new(DeciderChain::new(secrets, EgressPolicy::from_env()));
    let fixtures = Arc::new(fixtures);

    let mut report = String::new();
    report.push_str(&format!(
        "# Decider 評測報告\n\n- 產生時間：{}\n- 樣本：`{}`，共 {} 筆\n- 評測指標：以「Agent 是否該被喚醒」為單位的 micro precision / recall / F1；完全命中率（喚醒集合完全正確的比例）；模式準確率（none / single / parallel / discussion）。\n\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC"),
        args.fixtures.display(),
        fixtures.len()
    ));
    let mut raw_all = serde_json::Map::new();
    let mut summary_rows = Vec::new();

    for rec in &records {
        println!("evaluating {} …", rec.config.name);
        let started = Instant::now();
        let outcomes: Vec<Outcome> = stream::iter(0..fixtures.len())
            .map(|i| {
                let chain = chain.clone();
                let fixtures = fixtures.clone();
                let rec = rec.clone();
                async move {
                    let f = &fixtures[i];
                    let (input, _ids) = to_input(f);
                    let t = Instant::now();
                    // One retry on transient failures (rate limits / network) so the report measures quality.
                    let mut res = chain.call_one(&rec, &input).await;
                    if matches!(&res, Err(e) if matches!(e.kind(), "rate_limited" | "network" | "timeout" | "server")) {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        res = chain.call_one(&rec, &input).await;
                    }
                    let latency_ms = t.elapsed().as_millis() as u64;
                    let fid = f.id.clone().unwrap_or_else(|| format!("#{}", i + 1));
                    let category = f.category.clone().unwrap_or_else(|| "-".into());
                    match res {
                        Ok(call) => Outcome {
                            fixture: fid,
                            category,
                            ok: true,
                            error: None,
                            latency_ms: call.latency_ms,
                            scores: f
                                .candidates
                                .iter()
                                .zip(call.decision.scores.iter())
                                .map(|(c, (_, s))| (c.handle.clone(), *s))
                                .collect(),
                            interaction: Some(call.decision.interaction.as_str().into()),
                            interaction_confidence: call.decision.interaction_confidence,
                            expected_wake: f.expected_wake.clone(),
                            expected_mode: f.expected_mode.clone(),
                        },
                        Err(e) => Outcome {
                            fixture: fid,
                            category,
                            ok: false,
                            error: Some(e.to_string()),
                            latency_ms,
                            scores: f.candidates.iter().map(|c| (c.handle.clone(), 0.0)).collect(),
                            interaction: None,
                            interaction_confidence: 0.0,
                            expected_wake: f.expected_wake.clone(),
                            expected_mode: f.expected_mode.clone(),
                        },
                    }
                }
            })
            .buffered(args.concurrency.max(1))
            .collect()
            .await;
        let elapsed = started.elapsed().as_secs_f64();
        let errors = outcomes.iter().filter(|o| !o.ok).count();
        let mut lat: Vec<u64> = outcomes.iter().filter(|o| o.ok).map(|o| o.latency_ms).collect();
        let p50 = percentile(&mut lat, 0.5);
        let p95 = percentile(&mut lat, 0.95);
        let cfg = &rec.config;

        // Sweeps.
        let mut sweep = Vec::new();
        let mut t = 0.05f32;
        while t <= 0.951 {
            sweep.push((t, evaluate(&outcomes, cfg, t, cfg.t_discuss, None)));
            t += 0.05;
        }
        sweep.push((0.99, evaluate(&outcomes, cfg, 0.99, cfg.t_discuss, None)));
        let best = sweep
            .iter()
            .max_by(|a, b| {
                a.1.f1()
                    .partial_cmp(&b.1.f1())
                    .unwrap()
                    .then(a.1.precision().partial_cmp(&b.1.precision()).unwrap())
                    .then(b.0.partial_cmp(&a.0).unwrap())
            })
            .copied()
            .unwrap();
        let mut dsweep = Vec::new();
        let mut td = 0.3f32;
        while td <= 0.951 {
            dsweep.push((td, evaluate(&outcomes, cfg, best.0, td, None)));
            td += 0.05;
        }
        let best_d = dsweep
            .iter()
            .max_by(|a, b| a.1.mode_rate().partial_cmp(&b.1.mode_rate()).unwrap().then(a.0.partial_cmp(&b.0).unwrap()))
            .copied()
            .unwrap();
        let at_initial = evaluate(&outcomes, cfg, cfg.t_wake, cfg.t_discuss, None);
        let at_best = evaluate(&outcomes, cfg, best.0, best_d.0, None);

        report.push_str(&format!(
            "## {}\n\n- protocol：`{}`，model：`{}`\n- 錯誤：{} / {}，總耗時 {:.1}s，延遲 p50 {} ms / p95 {} ms\n\n",
            cfg.name,
            cfg.protocol,
            cfg.model,
            errors,
            outcomes.len(),
            elapsed,
            p50,
            p95
        ));
        report.push_str("| 門檻 | Precision | Recall | F1 | 完全命中 | 模式準確率 |\n|---|---|---|---|---|---|\n");
        report.push_str(&format!(
            "| 初值 T_WAKE={:.2}, T_DISCUSS={:.2} | {} | {} | {} | {} | {} |\n",
            cfg.t_wake,
            cfg.t_discuss,
            pct(at_initial.precision()),
            pct(at_initial.recall()),
            pct(at_initial.f1()),
            pct(at_initial.exact_rate()),
            pct(at_initial.mode_rate())
        ));
        report.push_str(&format!(
            "| **建議 T_WAKE={:.2}, T_DISCUSS={:.2}** | **{}** | **{}** | **{}** | **{}** | **{}** |\n\n",
            best.0,
            best_d.0,
            pct(at_best.precision()),
            pct(at_best.recall()),
            pct(at_best.f1()),
            pct(at_best.exact_rate()),
            pct(at_best.mode_rate())
        ));
        // Per category.
        let mut cats: Vec<String> = outcomes.iter().map(|o| o.category.clone()).collect();
        cats.sort();
        cats.dedup();
        report.push_str("依類別（建議門檻）：\n\n| 類別 | 筆數 | Precision | Recall | 完全命中 | 模式準確率 |\n|---|---|---|---|---|---|\n");
        for c in &cats {
            let m = evaluate(&outcomes, cfg, best.0, best_d.0, Some(c));
            report.push_str(&format!(
                "| {c} | {} | {} | {} | {} | {} |\n",
                m.n,
                pct(m.precision()),
                pct(m.recall()),
                pct(m.exact_rate()),
                pct(m.mode_rate())
            ));
        }
        report.push_str("\n<details><summary>T_WAKE 掃描</summary>\n\n| T_WAKE | Precision | Recall | F1 | 完全命中 |\n|---|---|---|---|---|\n");
        for (t, m) in &sweep {
            report.push_str(&format!(
                "| {t:.2} | {} | {} | {} | {} |\n",
                pct(m.precision()),
                pct(m.recall()),
                pct(m.f1()),
                pct(m.exact_rate())
            ));
        }
        report.push_str("\n</details>\n\n<details><summary>T_DISCUSS 掃描（T_WAKE 取建議值）</summary>\n\n| T_DISCUSS | 模式準確率 |\n|---|---|\n");
        for (t, m) in &dsweep {
            report.push_str(&format!("| {t:.2} | {} |\n", pct(m.mode_rate())));
        }
        report.push_str("\n</details>\n\n");
        // Misses at the suggested thresholds.
        let mut cfg2 = cfg.clone();
        cfg2.t_wake = best.0;
        cfg2.t_discuss = best_d.0;
        let mut misses = Vec::new();
        for (o, f) in outcomes.iter().zip(fixtures.iter()) {
            let m = evaluate(std::slice::from_ref(o), &cfg2, best.0, best_d.0, None);
            if m.exact == 0 || m.mode_ok == 0 {
                let scores = o.scores.iter().map(|(h, s)| format!("{h}={s:.2}")).collect::<Vec<_>>().join(" ");
                misses.push(format!(
                    "| {} | {} | {} | {} / {} | {} {} |",
                    o.fixture,
                    f.message.body.chars().take(40).collect::<String>().replace('|', "／"),
                    o.expected_wake.join(","),
                    o.expected_mode,
                    o.interaction.clone().unwrap_or_default(),
                    scores,
                    o.error
                        .clone()
                        .map(|e| format!("（錯誤：{}）", e.chars().take(60).collect::<String>()))
                        .unwrap_or_default()
                ));
            }
        }
        report.push_str(&format!("<details><summary>建議門檻下的錯誤樣本（{} 筆）</summary>\n\n| 樣本 | 訊息 | 應喚醒 | 應有模式 / 判斷互動 | 分數 |\n|---|---|---|---|---|\n", misses.len()));
        for m in misses.iter().take(40) {
            report.push_str(m);
            report.push('\n');
        }
        report.push_str("\n</details>\n\n");

        summary_rows.push(format!(
            "| {} | {} | {} | {} | {} | {} ms | {:.2} / {:.2} |",
            cfg.name,
            pct(at_best.precision()),
            pct(at_best.recall()),
            pct(at_best.f1()),
            pct(at_best.mode_rate()),
            p50,
            best.0,
            best_d.0
        ));
        raw_all.insert(cfg.name.clone(), serde_json::to_value(&outcomes)?);
        println!(
            "  {}: P={} R={} F1={} mode={} p50={}ms errors={} → suggest T_WAKE={:.2} T_DISCUSS={:.2}",
            cfg.name,
            pct(at_best.precision()),
            pct(at_best.recall()),
            pct(at_best.f1()),
            pct(at_best.mode_rate()),
            p50,
            errors,
            best.0,
            best_d.0
        );
    }

    let mut head = String::from(
        "## 總覽（各自的建議門檻）\n\n| 設定 | Precision | Recall | F1 | 模式準確率 | 延遲 p50 | 建議 T_WAKE / T_DISCUSS |\n|---|---|---|---|---|---|---|\n",
    );
    for r in &summary_rows {
        head.push_str(r);
        head.push('\n');
    }
    head.push('\n');
    let insert_at = report.find("## ").unwrap_or(report.len());
    report.insert_str(insert_at, &head);

    if let Some(out) = &args.out {
        std::fs::write(out, &report)?;
        println!("report written to {}", out.display());
    } else {
        println!("\n{report}");
    }
    if let Some(raw) = &args.raw {
        std::fs::write(raw, serde_json::to_string_pretty(&raw_all)?)?;
    }
    Ok(())
}

/// `ac-cli decider seed`: the same default list a new workspace gets.
pub async fn seed(store: &Store, workspace: Uuid) -> anyhow::Result<usize> {
    anyhow::ensure!(
        store.list_decider_configs(workspace).await?.is_empty(),
        "workspace already has decider configs; edit them in the settings page instead"
    );
    let master = std::env::var("CREWCONVO_MASTER_KEY").map_err(|_| anyhow::anyhow!("CREWCONVO_MASTER_KEY not set"))?;
    let secrets = SecretBox::from_base64(&master).map_err(anyhow::Error::msg)?;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let mut list = vec![(ac_decider::defaults::decisions_api(), env("GEMINI_API_KEY"))];
    if let Some(k) = env("OPENAI_API_KEY") {
        list.push((ac_decider::defaults::openai_structured(), Some(k)));
    }
    let n = list.len();
    for (i, (d, key)) in list.into_iter().enumerate() {
        let w = ac_store::DeciderConfigWrite {
            name: Some(d.name.into()),
            protocol: Some(d.protocol),
            endpoint_url: Some(d.endpoint_url.into()),
            api_key: key.map(|k| Some((secrets.encrypt(&k), last4(&k)))),
            auth_header: Some(d.auth_header.into()),
            auth_prefix: Some(d.auth_prefix.into()),
            model: Some(d.model.into()),
            input_field: Some("input".into()),
            timeout_ms: Some(if d.protocol == DeciderProtocol::DecisionsApi { 1500 } else { 8000 }),
            t_wake: Some(d.t_wake),
            t_discuss: Some(d.t_discuss),
            max_wake: Some(4),
            extra_headers: Some(json!({})),
            extra_body: Some(d.extra_body),
            enabled: Some(true),
            priority: Some(i as i32),
        };
        store.create_decider_config(workspace, &w, None).await?;
    }
    Ok(n)
}
