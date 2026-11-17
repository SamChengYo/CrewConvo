use ac_core::AppResult;
use ac_core::model::{Member, MemberKind};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::rows::{MEMBER_COLS, MemberRow};
use crate::{DbResultExt, Store};

impl Store {
    pub async fn create_human(
        &self,
        workspace_id: Uuid,
        handle: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> AppResult<Member> {
        let mut tx = self.pool.begin().await.db()?;
        let member =
            Store::insert_member(&mut tx, workspace_id, MemberKind::Human, handle, display_name, is_admin).await?;
        sqlx::query("INSERT INTO human_credentials (member_id, password_hash) VALUES ($1, $2)")
            .bind(member.id)
            .bind(password_hash)
            .execute(&mut *tx)
            .await
            .db()?;
        tx.commit().await.db()?;
        Ok(member)
    }

    pub async fn credentials_by_handle(&self, workspace_id: Uuid, handle: &str) -> AppResult<Option<(Member, String)>> {
        let sql = format!(
            "SELECT {MEMBER_COLS}, hc.password_hash FROM members m \
             JOIN human_credentials hc ON hc.member_id = m.id \
             WHERE m.workspace_id = $1 AND lower(m.handle) = lower($2) AND m.deactivated_at IS NULL"
        );
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            member: MemberRow,
            password_hash: String,
        }
        let row: Option<Row> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).bind(handle).fetch_optional(&self.pool).await.db()?;
        match row {
            Some(r) => Ok(Some((r.member.into_model()?, r.password_hash))),
            None => Ok(None),
        }
    }

    pub async fn create_session(&self, token_hash: &str, member_id: Uuid, expires_at: DateTime<Utc>) -> AppResult<()> {
        sqlx::query("INSERT INTO sessions (token_hash, member_id, expires_at) VALUES ($1, $2, $3)")
            .bind(token_hash)
            .bind(member_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn session_member(&self, token_hash: &str) -> AppResult<Option<Member>> {
        let sql = format!(
            "SELECT {MEMBER_COLS} FROM sessions s JOIN members m ON m.id = s.member_id \
             WHERE s.token_hash = $1 AND s.expires_at > now() AND m.deactivated_at IS NULL"
        );
        let row: Option<MemberRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(token_hash).fetch_optional(&self.pool).await.db()?;
        row.map(MemberRow::into_model).transpose()
    }

    pub async fn delete_session(&self, token_hash: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash).execute(&self.pool).await.db()?;
        Ok(())
    }
}
