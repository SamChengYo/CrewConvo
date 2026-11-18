//! Local accounts and sessions (spec 10: local account + session cookie).

use ac_core::mention::validate_handle;
use ac_core::model::{ConversationKind, Member, Visibility};
use ac_core::{AppError, AppResult};
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Argon2, PasswordHash};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use crate::Service;

pub const SESSION_DAYS: i64 = 30;

pub fn hash_password(password: &str) -> AppResult<String> {
    let h: PasswordHash = Argon2::default().hash_password(password.as_bytes()).map_err(AppError::internal)?;
    Ok(h.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordVerifier::<str>::verify_password(&Argon2::default(), password.as_bytes(), hash).is_ok()
}

/// 244 bits of OS randomness, hex encoded.
pub fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Only token hashes are stored.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

impl Service {
    /// Registers a human. The first human of a deployment creates the workspace and becomes admin.
    pub async fn register(&self, handle: &str, display_name: &str, password: &str) -> AppResult<Member> {
        let handle = handle.trim();
        validate_handle(handle).map_err(AppError::BadRequest)?;
        if password.chars().count() < 8 {
            return Err(AppError::bad_request("密碼至少需要 8 個字元"));
        }
        let display_name = if display_name.trim().is_empty() { handle } else { display_name.trim() };
        let store = &self.inner.store;
        let ws = match store.default_workspace().await? {
            Some(ws) => ws,
            None => {
                let ws = store.create_workspace("CrewConvo").await?;
                self.seed_default_deciders(ws.id).await?;
                ws
            }
        };
        let first = store.count_humans(ws.id).await? == 0;
        let pw = hash_password(password)?;
        let member = store.create_human(ws.id, handle, display_name, &pw, first).await?;

        // Everyone joins #general.
        let general =
            store.list_public_channels(ws.id).await?.into_iter().find(|c| c.name.as_deref() == Some("general"));
        match general {
            Some(c) => {
                store.add_conversation_member(c.id, member.id, None).await?;
            }
            None => {
                store
                    .create_conversation(
                        ws.id,
                        ConversationKind::Channel,
                        Some("general"),
                        Some("全體成員的預設頻道"),
                        Visibility::Public,
                        member.id,
                        &[member.id],
                    )
                    .await?;
            }
        }
        Ok(member)
    }

    /// Verifies credentials and creates a session; returns (member, raw session token).
    pub async fn login(&self, handle: &str, password: &str) -> AppResult<(Member, String)> {
        let store = &self.inner.store;
        let ws = store.default_workspace().await?.ok_or(AppError::Unauthorized)?;
        let Some((member, hash)) = store.credentials_by_handle(ws.id, handle.trim()).await? else {
            // Spend comparable time to avoid user enumeration by timing.
            let _ = verify_password(password, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA");
            return Err(AppError::Unauthorized);
        };
        if !verify_password(password, &hash) {
            return Err(AppError::Unauthorized);
        }
        let token = new_token();
        store.create_session(&hash_token(&token), member.id, Utc::now() + Duration::days(SESSION_DAYS)).await?;
        Ok((member, token))
    }

    pub async fn session_member(&self, token: &str) -> AppResult<Option<Member>> {
        self.inner.store.session_member(&hash_token(token)).await
    }

    pub async fn logout(&self, token: &str) -> AppResult<()> {
        self.inner.store.delete_session(&hash_token(token)).await
    }
}

impl Service {
    /// Signs a person up with an e-mail address, a name and a password. The name is both what
    /// others see and what they type after `@` (spaces become `-`), so it must be unique.
    pub async fn register_account(&self, email: &str, name: &str, password: &str) -> AppResult<Member> {
        let email = email.trim();
        let parts: Vec<&str> = email.split("@").collect();
        let valid = parts.len() == 2
            && !parts[0].is_empty()
            && parts[1].contains(".")
            && !email.chars().any(char::is_whitespace)
            && email.len() <= 254;
        if !valid {
            return Err(AppError::bad_request("請輸入有效的電子信箱"));
        }
        let name = name.trim();
        let handle = ac_core::mention::handle_from_name(name)
            .ok_or_else(|| AppError::bad_request("名字需要 1 到 32 個字，可以用中文、英文與數字"))?;
        let store = &self.inner.store;
        if store.email_taken(email).await? {
            return Err(AppError::Conflict("這個信箱已經註冊過了，請直接登入".into()));
        }
        if let Some(ws) = store.default_workspace().await?
            && store.member_by_handle(ws.id, &handle).await?.is_some()
        {
            return Err(AppError::Conflict(format!("「{name}」這個名字已經有人使用，請換一個")));
        }
        let member = self.register(&handle, name, password).await?;
        store.set_human_email(member.id, email).await?;
        Ok(member)
    }
}
