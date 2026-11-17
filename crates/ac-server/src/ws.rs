//! Realtime events for the web UI: `GET /api/ws` (session cookie).

use std::collections::HashSet;

use ac_core::bus::RealtimeEvent;
use ac_core::model::Member;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::session::CurrentMember;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/ws", get(upgrade))
}

async fn upgrade(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(s, m, socket))
}

async fn run(s: AppState, member: Member, mut socket: WebSocket) {
    let mut rx = s.service.engine().bus().subscribe();
    let mut convs: HashSet<uuid::Uuid> = match s.service.store().member_conversation_ids(member.id).await {
        Ok(v) => v.into_iter().collect(),
        Err(_) => return,
    };
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let ev = match ev {
                    Ok(ev) => ev,
                    Err(RecvError::Lagged(_)) => {
                        if socket.send(Message::Text(r#"{"type":"resync"}"#.into())).await.is_err() { return; }
                        continue;
                    }
                    Err(RecvError::Closed) => return,
                };
                let deliver = match &ev {
                    RealtimeEvent::MembershipChanged { conversation_id, member_id, joined } => {
                        if *member_id == member.id {
                            if *joined { convs.insert(*conversation_id); } else { convs.remove(conversation_id); }
                            true
                        } else {
                            convs.contains(conversation_id)
                        }
                    }
                    RealtimeEvent::AgentsChanged { workspace_id } => *workspace_id == member.workspace_id,
                    other => other.conversation_id().map(|c| convs.contains(&c)).unwrap_or(false),
                };
                if deliver {
                    let Ok(text) = serde_json::to_string(&ev) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            }
        }
    }
}
