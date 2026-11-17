//! Decider configurations and decision audit records (spec 6.2).

use ac_core::model::{DeciderConfig, DeciderProtocol, DecisionRecord};
use ac_core::{AppError, AppResult, new_id};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::AssertSqlSafe;
use sqlx::types::Json;
use uuid::Uuid;

use crate::rows::collect;
use crate::{DbResultExt, Store};

/// A config plus its encrypted key (server-internal only).
#[derive(Debug, Clone)]
pub struct DeciderConfigRecord {
    pub config: DeciderConfig,
    pub api_key_encrypted: Option<String>,
}

/// Fields for create / update. `None` keeps the current value on update.
#[derive(Debug, Clone, Default)]
pub struct DeciderConfigWrite {
    pub name: Option<String>,
    pub protocol: Option<DeciderProtocol>,
    pub endpoint_url: Option<String>,
    /// `Some(Some(enc))` sets, `Some(None)` clears.
    pub api_key: Option<Option<(String, String)>>,
    pub auth_header: Option<String>,
    pub auth_prefix: Option<String>,
    pub model: Option<String>,
    pub input_field: Option<String>,
    pub timeout_ms: Option<i32>,
    pub t_wake: Option<f32>,
    pub t_discuss: Option<f32>,
    pub max_wake: Option<i32>,
    pub extra_headers: Option<Value>,
    pub extra_body: Option<Value>,
    pub enabled: Option<bool>,
    pub priority: Option<i32>,
}

const COLS: &str = "id, workspace_id, name, protocol, endpoint_url, api_key_encrypted, api_key_last4, auth_header, \
     auth_prefix, model, input_field, timeout_ms, t_wake, t_discuss, max_wake, extra_headers, extra_body, enabled, \
     priority, updated_by, updated_at";

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    protocol: String,
    endpoint_url: String,
    api_key_encrypted: Option<String>,
    api_key_last4: Option<String>,
    auth_header: String,
    auth_prefix: String,
    model: String,
    input_field: String,
    timeout_ms: i32,
    t_wake: f32,
    t_discuss: f32,
    max_wake: i32,
    extra_headers: Json<Value>,
    extra_body: Json<Value>,
    enabled: bool,
    priority: i32,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
}

impl Row {
    fn into_record(self) -> AppResult<DeciderConfigRecord> {
        Ok(DeciderConfigRecord {
            config: DeciderConfig {
                id: self.id,
                workspace_id: self.workspace_id,
                name: self.name,
                protocol: self.protocol.parse()?,
                endpoint_url: self.endpoint_url,
                has_api_key: self.api_key_encrypted.is_some(),
                api_key_last4: self.api_key_last4,
                auth_header: self.auth_header,
                auth_prefix: self.auth_prefix,
                model: self.model,
                input_field: self.input_field,
                timeout_ms: self.timeout_ms,
                t_wake: self.t_wake,
                t_discuss: self.t_discuss,
                max_wake: self.max_wake,
                extra_headers: self.extra_headers.0,
                extra_body: self.extra_body.0,
                enabled: self.enabled,
                priority: self.priority,
                updated_by: self.updated_by,
                updated_at: self.updated_at,
            },
            api_key_encrypted: self.api_key_encrypted,
        })
    }
}

impl Store {
    pub async fn list_decider_configs(&self, workspace_id: Uuid) -> AppResult<Vec<DeciderConfigRecord>> {
        let sql = format!("SELECT {COLS} FROM decider_configs WHERE workspace_id = $1 ORDER BY priority, updated_at");
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).fetch_all(&self.pool).await.db()?;
        collect(rows, Row::into_record)
    }

    pub async fn get_decider_config(&self, id: Uuid) -> AppResult<Option<DeciderConfigRecord>> {
        let sql = format!("SELECT {COLS} FROM decider_configs WHERE id = $1");
        let row: Option<Row> = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(Row::into_record).transpose()
    }

    pub async fn create_decider_config(
        &self,
        workspace_id: Uuid,
        w: &DeciderConfigWrite,
        updated_by: Option<Uuid>,
    ) -> AppResult<DeciderConfigRecord> {
        let need = |o: &Option<String>, f: &str| o.clone().ok_or_else(|| AppError::bad_request(format!("{f} 必填")));
        let (enc, last4) = match &w.api_key {
            Some(Some((e, l))) => (Some(e.clone()), Some(l.clone())),
            _ => (None, None),
        };
        let priority = match w.priority {
            Some(p) => p,
            None => {
                let (p,): (Option<i32>,) =
                    sqlx::query_as("SELECT max(priority) FROM decider_configs WHERE workspace_id = $1")
                        .bind(workspace_id)
                        .fetch_one(&self.pool)
                        .await
                        .db()?;
                p.map(|p| p + 1).unwrap_or(0)
            }
        };
        let sql = format!(
            "INSERT INTO decider_configs (id, workspace_id, name, protocol, endpoint_url, api_key_encrypted, \
             api_key_last4, auth_header, auth_prefix, model, input_field, timeout_ms, t_wake, t_discuss, max_wake, \
             extra_headers, extra_body, enabled, priority, updated_by) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20) RETURNING {COLS}"
        );
        let row: Row = sqlx::query_as(AssertSqlSafe(sql))
            .bind(new_id())
            .bind(workspace_id)
            .bind(need(&w.name, "name")?)
            .bind(w.protocol.ok_or_else(|| AppError::bad_request("protocol 必填"))?.as_str())
            .bind(need(&w.endpoint_url, "endpoint_url")?)
            .bind(enc)
            .bind(last4)
            .bind(need(&w.auth_header, "auth_header")?)
            .bind(w.auth_prefix.clone().unwrap_or_default())
            .bind(need(&w.model, "model")?)
            .bind(w.input_field.clone().unwrap_or_else(|| "input".into()))
            .bind(w.timeout_ms.unwrap_or(1500))
            .bind(w.t_wake.unwrap_or(0.6))
            .bind(w.t_discuss.unwrap_or(0.7))
            .bind(w.max_wake.unwrap_or(4))
            .bind(Json(w.extra_headers.clone().unwrap_or_else(|| serde_json::json!({}))))
            .bind(Json(w.extra_body.clone().unwrap_or_else(|| serde_json::json!({}))))
            .bind(w.enabled.unwrap_or(true))
            .bind(priority)
            .bind(updated_by)
            .fetch_one(&self.pool)
            .await
            .db()?;
        row.into_record()
    }

    pub async fn update_decider_config(
        &self,
        id: Uuid,
        w: &DeciderConfigWrite,
        updated_by: Option<Uuid>,
    ) -> AppResult<DeciderConfigRecord> {
        let (set_key, enc, last4) = match &w.api_key {
            Some(Some((e, l))) => (true, Some(e.clone()), Some(l.clone())),
            Some(None) => (true, None, None),
            None => (false, None, None),
        };
        let sql = format!(
            "UPDATE decider_configs SET \
               name = COALESCE($2, name), protocol = COALESCE($3, protocol), endpoint_url = COALESCE($4, endpoint_url), \
               api_key_encrypted = CASE WHEN $5 THEN $6 ELSE api_key_encrypted END, \
               api_key_last4 = CASE WHEN $5 THEN $7 ELSE api_key_last4 END, \
               auth_header = COALESCE($8, auth_header), auth_prefix = COALESCE($9, auth_prefix), \
               model = COALESCE($10, model), input_field = COALESCE($11, input_field), \
               timeout_ms = COALESCE($12, timeout_ms), t_wake = COALESCE($13, t_wake), \
               t_discuss = COALESCE($14, t_discuss), max_wake = COALESCE($15, max_wake), \
               extra_headers = COALESCE($16, extra_headers), extra_body = COALESCE($17, extra_body), \
               enabled = COALESCE($18, enabled), priority = COALESCE($19, priority), \
               updated_by = $20, updated_at = now() \
             WHERE id = $1 RETURNING {COLS}"
        );
        let row: Option<Row> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(&w.name)
            .bind(w.protocol.map(|p| p.as_str()))
            .bind(&w.endpoint_url)
            .bind(set_key)
            .bind(enc)
            .bind(last4)
            .bind(&w.auth_header)
            .bind(&w.auth_prefix)
            .bind(&w.model)
            .bind(&w.input_field)
            .bind(w.timeout_ms)
            .bind(w.t_wake)
            .bind(w.t_discuss)
            .bind(w.max_wake)
            .bind(w.extra_headers.as_ref().map(Json))
            .bind(w.extra_body.as_ref().map(Json))
            .bind(w.enabled)
            .bind(w.priority)
            .bind(updated_by)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        row.ok_or_else(|| AppError::not_found("decider config"))?.into_record()
    }

    pub async fn delete_decider_config(&self, id: Uuid) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.db()?;
        sqlx::query("UPDATE decisions SET decider_config_id = NULL WHERE decider_config_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .db()?;
        let r = sqlx::query("DELETE FROM decider_configs WHERE id = $1").bind(id).execute(&mut *tx).await.db()?;
        tx.commit().await.db()?;
        Ok(r.rows_affected() > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_decision(&self, d: &DecisionRecord) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO decisions (id, trigger_message_id, decider_config_id, source, reason, attempts, request, \
             response, scores, interaction, interaction_confidence, woken, mode, latency_ms) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(d.id)
        .bind(d.trigger_message_id)
        .bind(d.decider_config_id)
        .bind(&d.source)
        .bind(&d.reason)
        .bind(Json(&d.attempts))
        .bind(d.request.as_ref().map(Json))
        .bind(d.response.as_ref().map(Json))
        .bind(d.scores.as_ref().map(Json))
        .bind(&d.interaction)
        .bind(d.interaction_confidence)
        .bind(&d.woken)
        .bind(&d.mode)
        .bind(d.latency_ms)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn decisions_for_message(&self, message_id: Uuid) -> AppResult<Vec<DecisionRecord>> {
        #[derive(sqlx::FromRow)]
        struct DecRow {
            id: Uuid,
            trigger_message_id: Uuid,
            decider_config_id: Option<Uuid>,
            source: String,
            reason: Option<String>,
            attempts: Json<Value>,
            request: Option<Json<Value>>,
            response: Option<Json<Value>>,
            scores: Option<Json<Value>>,
            interaction: Option<String>,
            interaction_confidence: Option<f32>,
            woken: Vec<Uuid>,
            mode: Option<String>,
            latency_ms: Option<i32>,
            created_at: DateTime<Utc>,
        }
        let rows: Vec<DecRow> = sqlx::query_as(
            "SELECT id, trigger_message_id, decider_config_id, source, reason, attempts, request, response, scores, \
             interaction, interaction_confidence, woken, mode, latency_ms, created_at \
             FROM decisions WHERE trigger_message_id = $1 ORDER BY created_at",
        )
        .bind(message_id)
        .fetch_all(&self.pool)
        .await
        .db()?;
        Ok(rows
            .into_iter()
            .map(|r| DecisionRecord {
                id: r.id,
                trigger_message_id: r.trigger_message_id,
                decider_config_id: r.decider_config_id,
                source: r.source,
                reason: r.reason,
                attempts: r.attempts.0,
                request: r.request.map(|j| j.0),
                response: r.response.map(|j| j.0),
                scores: r.scores.map(|j| j.0),
                interaction: r.interaction,
                interaction_confidence: r.interaction_confidence,
                woken: r.woken,
                mode: r.mode,
                latency_ms: r.latency_ms,
                created_at: r.created_at,
            })
            .collect())
    }

    /// Per-config call statistics over the most recent `window` attempts (from `decisions.attempts`).
    /// Returns (config_id, calls, failures, avg latency ms, p95 latency ms).
    pub async fn decider_stats(&self, workspace_id: Uuid, window: i64) -> AppResult<Vec<(Uuid, i64, i64, f64, f64)>> {
        let rows: Vec<(Uuid, i64, i64, Option<f64>, Option<f64>)> = sqlx::query_as(
            "WITH a AS ( \
               SELECT (att->>'config_id')::uuid AS config_id, (att->>'ok')::boolean AS ok, \
                      (att->>'latency_ms')::float8 AS latency, d.created_at, \
                      row_number() OVER (PARTITION BY att->>'config_id' ORDER BY d.created_at DESC) AS rn \
               FROM decisions d CROSS JOIN LATERAL jsonb_array_elements(d.attempts) att \
               WHERE att ? 'config_id' AND (att->>'config_id')::uuid IN \
                     (SELECT id FROM decider_configs WHERE workspace_id = $1)) \
             SELECT config_id, count(*), count(*) FILTER (WHERE NOT ok), avg(latency), \
                    percentile_cont(0.95) WITHIN GROUP (ORDER BY latency) \
             FROM a WHERE rn <= $2 GROUP BY config_id",
        )
        .bind(workspace_id)
        .bind(window)
        .fetch_all(&self.pool)
        .await
        .db()?;
        Ok(rows.into_iter().map(|(id, n, f, avg, p95)| (id, n, f, avg.unwrap_or(0.0), p95.unwrap_or(0.0))).collect())
    }
}
