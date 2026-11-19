//! Conversations of coding-agent projects: renaming and archiving channels, and the project
//! endpoints being absent on a server that does not manage local coding agents.

mod common;

use ac_core::AppError;
use ac_core::model::Visibility;
use common::fixture;
use serde_json::json;

#[tokio::test]
async fn a_channel_can_be_renamed_by_its_members() {
    let fx = fixture().await;
    let svc = fx.svc();
    let shop = svc.create_channel(&fx.human, "shop", None, Visibility::Private, &[]).await.unwrap();
    let api = svc.create_channel(&fx.human, "api", None, Visibility::Private, &[]).await.unwrap();

    let renamed = svc.rename_channel(&fx.human, shop.id, " #shop-frontend ").await.unwrap();
    assert_eq!(renamed.name.as_deref(), Some("shop-frontend"));
    assert_eq!(fx.store().get_conversation(shop.id).await.unwrap().unwrap().name.as_deref(), Some("shop-frontend"));

    // A name another channel has, and a name that is not one.
    assert!(matches!(svc.rename_channel(&fx.human, api.id, "Shop-Frontend").await, Err(AppError::Conflict(_))));
    assert!(matches!(svc.rename_channel(&fx.human, api.id, "two words").await, Err(AppError::BadRequest(_))));

    // Somebody who is not in the channel cannot rename it.
    let bob = fx.second_human("bob", false).await;
    assert!(svc.rename_channel(&bob, api.id, "mine").await.is_err());
}

#[tokio::test]
async fn an_archived_channel_leaves_the_lists_and_keeps_its_messages() {
    let fx = fixture().await;
    let svc = fx.svc();
    let c = svc.create_channel(&fx.human, "old-project", None, Visibility::Private, &[]).await.unwrap();
    let posted = svc.post_message(&fx.human, None, c.id, "記得這個", None).await.unwrap();

    svc.archive_channel(&fx.human, c.id).await.unwrap();
    let listed = svc.list_conversations(&fx.human).await.unwrap();
    assert!(!listed.iter().any(|i| i.conversation.id == c.id));
    // Nothing was deleted: the conversation and its message are still there…
    assert!(fx.store().get_conversation(c.id).await.unwrap().unwrap().archived_at.is_some());
    assert!(fx.store().get_message(posted.message.id).await.unwrap().is_some());
    // …but it takes no more messages.
    assert!(svc.post_message(&fx.human, None, c.id, "還在嗎", None).await.is_err());
    // Archiving twice is not an error.
    svc.archive_channel(&fx.human, c.id).await.unwrap();

    // The everyone-channel stays.
    let general = svc.create_channel(&fx.human, "general", None, Visibility::Public, &[]).await;
    if let Ok(g) = general {
        assert!(matches!(svc.archive_channel(&fx.human, g.id).await, Err(AppError::BadRequest(_))));
    }
}

#[tokio::test]
async fn project_endpoints_do_not_exist_without_local_agents() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let client = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let handle = format!("u{}", &id[..12]);
    let signed_in = client
        .post(format!("{base}/api/auth/register"))
        .json(&json!({ "handle": handle, "display_name": "U", "password": "long-enough-password" }))
        .send()
        .await
        .unwrap();
    assert!(signed_in.status().is_success(), "register → {}", signed_in.status());
    for (method, path) in [
        ("POST", "/api/local-agents/projects"),
        ("POST", "/api/local-agents/projects/check"),
        ("PUT", "/api/local-agents/projects/name"),
        ("POST", "/api/local-agents/conversations/00000000-0000-0000-0000-000000000001/archive"),
    ] {
        let r = client
            .request(method.parse().unwrap(), format!("{base}{path}"))
            .json(&json!({ "path": "/tmp/x", "name": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 404, "{method} {path}");
    }
}
