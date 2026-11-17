use ac_core::model::{Member, MemberKind, Workspace};
use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::rows::{MEMBER_COLS, MemberRow, collect};
use crate::{DbResultExt, Store};

pub(crate) const SYSTEM_HANDLE: &str = "system";

impl Store {
    pub async fn create_workspace(&self, name: &str) -> AppResult<Workspace> {
        let mut tx = self.pool.begin().await.db()?;
        let id = new_id();
        let (created_at,): (DateTime<Utc>,) =
            sqlx::query_as("INSERT INTO workspaces (id, name) VALUES ($1, $2) RETURNING created_at")
                .bind(id)
                .bind(name)
                .fetch_one(&mut *tx)
                .await
                .db()?;
        sqlx::query(
            "INSERT INTO members (id, workspace_id, kind, handle, display_name) VALUES ($1, $2, 'system', $3, '系統')",
        )
        .bind(new_id())
        .bind(id)
        .bind(SYSTEM_HANDLE)
        .execute(&mut *tx)
        .await
        .db()?;
        tx.commit().await.db()?;
        Ok(Workspace { id, name: name.to_string(), created_at })
    }

    /// The oldest workspace (v1 runs one workspace per deployment).
    pub async fn default_workspace(&self) -> AppResult<Option<Workspace>> {
        let row: Option<(Uuid, String, DateTime<Utc>)> =
            sqlx::query_as("SELECT id, name, created_at FROM workspaces ORDER BY id LIMIT 1")
                .fetch_optional(&self.pool)
                .await
                .db()?;
        Ok(row.map(|(id, name, created_at)| Workspace { id, name, created_at }))
    }

    pub async fn system_member(&self, workspace_id: Uuid) -> AppResult<Member> {
        let sql = format!("SELECT {MEMBER_COLS} FROM members m WHERE m.workspace_id = $1 AND m.kind = 'system'");
        let row: MemberRow = sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).fetch_one(&self.pool).await.db()?;
        row.into_model()
    }

    pub async fn get_member(&self, id: Uuid) -> AppResult<Option<Member>> {
        let sql = format!("SELECT {MEMBER_COLS} FROM members m WHERE m.id = $1");
        let row: Option<MemberRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(MemberRow::into_model).transpose()
    }

    pub async fn get_members(&self, ids: &[Uuid]) -> AppResult<Vec<Member>> {
        let sql = format!("SELECT {MEMBER_COLS} FROM members m WHERE m.id = ANY($1) ORDER BY m.handle");
        let rows: Vec<MemberRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(ids).fetch_all(&self.pool).await.db()?;
        collect(rows, MemberRow::into_model)
    }

    pub async fn member_by_handle(&self, workspace_id: Uuid, handle: &str) -> AppResult<Option<Member>> {
        let sql =
            format!("SELECT {MEMBER_COLS} FROM members m WHERE m.workspace_id = $1 AND lower(m.handle) = lower($2)");
        let row: Option<MemberRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).bind(handle).fetch_optional(&self.pool).await.db()?;
        row.map(MemberRow::into_model).transpose()
    }

    /// Members whose (lower-cased) handle is in `handles`.
    pub async fn members_by_handles(&self, workspace_id: Uuid, handles: &[String]) -> AppResult<Vec<Member>> {
        let sql = format!(
            "SELECT {MEMBER_COLS} FROM members m WHERE m.workspace_id = $1 AND lower(m.handle) = ANY($2) \
             AND m.deactivated_at IS NULL"
        );
        let rows: Vec<MemberRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).bind(handles).fetch_all(&self.pool).await.db()?;
        collect(rows, MemberRow::into_model)
    }

    pub async fn list_members(&self, workspace_id: Uuid) -> AppResult<Vec<Member>> {
        let sql = format!(
            "SELECT {MEMBER_COLS} FROM members m WHERE m.workspace_id = $1 AND m.kind <> 'system' \
             AND m.deactivated_at IS NULL ORDER BY m.kind, m.handle"
        );
        let rows: Vec<MemberRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).fetch_all(&self.pool).await.db()?;
        collect(rows, MemberRow::into_model)
    }

    pub async fn count_humans(&self, workspace_id: Uuid) -> AppResult<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM members WHERE workspace_id = $1 AND kind = 'human'")
            .bind(workspace_id)
            .fetch_one(&self.pool)
            .await
            .db()?;
        Ok(n)
    }

    pub(crate) async fn insert_member(
        tx: &mut sqlx::PgConnection,
        workspace_id: Uuid,
        kind: MemberKind,
        handle: &str,
        display_name: &str,
        is_admin: bool,
    ) -> AppResult<Member> {
        let id = new_id();
        let sql = format!(
            "INSERT INTO members AS m (id, workspace_id, kind, handle, display_name, is_admin) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING {MEMBER_COLS}"
        );
        let row: MemberRow = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(workspace_id)
            .bind(kind.as_str())
            .bind(handle)
            .bind(display_name)
            .bind(is_admin)
            .fetch_one(tx)
            .await
            .db()?;
        row.into_model()
    }
}
