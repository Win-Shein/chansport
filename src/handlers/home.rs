//! Home landing page: Admin Dashboard button + member buttons.

use axum::extract::State;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde_json::json;

use crate::models::Member;
use crate::state::{render, AppState, SessionUser};

async fn require_admin(state: &AppState, jar: &CookieJar) -> Option<SessionUser> {
    let u = state.current_user(jar).await?;
    if u.role == "ADMIN" {
        Some(u)
    } else {
        None
    }
}

pub async fn home(State(state): State<AppState>, jar: CookieJar) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };

    let members = sqlx::query_as::<_, Member>(
        "SELECT id, username, balance, COALESCE(commission,0) as commission, \
                COALESCE(bet_limit,100000) as bet_limit, COALESCE(status,'ACTIVE') as status, created_at \
         FROM users WHERE role='MEMBER' ORDER BY username ASC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let list: Vec<serde_json::Value> = members
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "username": m.username,
                "commission": m.commission,
            })
        })
        .collect();

    render(
        &state,
        "home.html",
        json!({
            "admin": { "username": admin.username },
            "members": list,
        }),
    )
}
