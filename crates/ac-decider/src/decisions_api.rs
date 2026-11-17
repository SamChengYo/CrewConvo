//! Protocol `decisions_api` (default: Google Gemini Decisions API).
//! One independent `noul` question per candidate plus one `choice` for the interaction mode.

use ac_core::model::Interaction;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    CallContext, DISCUSS_DESCRIPTION, DecideInput, DeciderError, Decision, FALSE_DESCRIPTION, INDEPENDENT_DESCRIPTION,
    INTERACTION_INSTRUCTIONS, TRUE_DESCRIPTION, agent_instructions, agent_key, context_text, merge_object, post_json,
};

const SAFETY_CODES: &[&str] = &["safety", "prohibited_content", "blocklist", "image_safety", "content_blocked"];

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Score {
        #[serde(default)]
        score: Option<f64>,
    },
}

pub fn build_request(cx: &CallContext<'_>, input: &DecideInput) -> Value {
    let mut questions = Map::new();
    for a in &input.candidates {
        questions.insert(
            agent_key(a.id),
            json!({
                "type": "noul",
                "instructions": agent_instructions(a),
                "criteria": { "true_description": TRUE_DESCRIPTION, "false_description": FALSE_DESCRIPTION },
            }),
        );
    }
    questions.insert(
        "interaction".into(),
        json!({
            "type": "choice",
            "instructions": INTERACTION_INSTRUCTIONS,
            "criteria": { "discuss": DISCUSS_DESCRIPTION, "independent": INDEPENDENT_DESCRIPTION },
        }),
    );
    let field = if cx.config.input_field.trim().is_empty() { "input" } else { cx.config.input_field.trim() };
    let mut body = Map::new();
    body.insert("model".into(), json!(cx.config.model));
    body.insert(field.into(), json!(context_text(input)));
    body.insert("questions".into(), Value::Object(questions));
    let mut body = Value::Object(body);
    merge_object(&mut body, &cx.config.extra_body, &["questions", field]);
    body
}

pub fn parse_response(input: &DecideInput, v: &Value) -> Result<Decision, DeciderError> {
    let answers =
        v.get("answers").and_then(Value::as_object).ok_or_else(|| DeciderError::Invalid("missing answers".into()))?;
    let mut scores = Vec::with_capacity(input.candidates.len());
    for a in &input.candidates {
        let s = match answers.get(&agent_key(a.id)).map(|x| serde_json::from_value::<Answer>(x.clone())) {
            Some(Ok(Answer::Noul { noul })) => noul,
            Some(Ok(Answer::Score { score })) => score.unwrap_or(0.0),
            Some(Ok(_)) | Some(Err(_)) => return Err(DeciderError::Invalid(format!("bad answer for @{}", a.handle))),
            None => 0.0,
        };
        scores.push((a.id, s.clamp(0.0, 1.0) as f32));
    }
    let (interaction, confidence) =
        match answers.get("interaction").map(|x| serde_json::from_value::<Answer>(x.clone())) {
            Some(Ok(Answer::Choice { choice, confidence })) => (
                if choice == "discuss" { Interaction::Discuss } else { Interaction::Independent },
                confidence.unwrap_or(0.0),
            ),
            _ => (Interaction::Independent, 0.0),
        };
    Ok(Decision { scores, interaction, interaction_confidence: confidence.clamp(0.0, 1.0) as f32 })
}

pub fn classify_error(status: u16, v: &Value) -> DeciderError {
    let code = v["error"]["code"].as_str().unwrap_or_default().to_string();
    let msg =
        v["error"]["message"].as_str().map(String::from).unwrap_or_else(|| v.to_string().chars().take(500).collect());
    match status {
        400 if SAFETY_CODES.contains(&code.as_str()) => DeciderError::Safety(format!("{code}: {msg}")),
        400 => DeciderError::BadRequest(format!("{code}: {msg}")),
        429 => DeciderError::RateLimited(msg),
        s if s >= 500 => DeciderError::Server(s, msg),
        s => DeciderError::BadRequest(format!("HTTP {s}: {msg}")),
    }
}

pub async fn call(cx: &CallContext<'_>, input: &DecideInput) -> Result<(Value, Value, Decision), DeciderError> {
    let request = build_request(cx, input);
    let (status, response) = post_json(cx, &request).await?;
    if !(200..300).contains(&status) {
        return Err(classify_error(status, &response));
    }
    let decision = parse_response(input, &response)?;
    Ok((request, response, decision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::model::{AgentOrigin, AgentProfile, AgentStatus, MemberKind};
    use uuid::Uuid;

    fn input() -> DecideInput {
        DecideInput {
            message: crate::DecideMessage {
                author_handle: "alice".into(),
                author_kind: MemberKind::Human,
                body: "慢查詢".into(),
            },
            recent: vec![],
            conversation: crate::DecideConversation { name: Some("ops".into()), topic: None, is_dm: false },
            candidates: vec![AgentProfile {
                id: Uuid::from_u128(7),
                handle: "dba".into(),
                display_name: "DBA".into(),
                origin: AgentOrigin::Builtin,
                description: "資料庫".into(),
                expertise: vec![],
                status: AgentStatus::Online,
            }],
        }
    }

    #[test]
    fn parses_official_shape() {
        let i = input();
        let key = agent_key(Uuid::from_u128(7));
        let v = json!({"answers": {
            key: {"type":"noul","noul":0.93,"confidence":0.93},
            "interaction": {"type":"choice","choice":"discuss","probabilities":{"discuss":0.8,"independent":0.2},"confidence":0.8}
        }});
        let d = parse_response(&i, &v).unwrap();
        assert!((d.scores[0].1 - 0.93).abs() < 1e-6);
        assert_eq!(d.interaction, Interaction::Discuss);
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(classify_error(400, &json!({"error":{"code":"safety","message":"x"}})).kind(), "safety");
        assert_eq!(classify_error(429, &json!({})).kind(), "rate_limited");
        assert_eq!(classify_error(503, &json!({})).kind(), "server");
    }
}
