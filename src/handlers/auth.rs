//! Login / logout / change-password handlers.

use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::handlers::clean_string;
use crate::models::User;
use crate::state::{clear_cookie, render, session_cookie, AppState, SessionUser};

pub async fn get_login(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(user) = state.current_user(&jar).await {
        return if user.role == "ADMIN" {
            Redirect::to("/admin/members").into_response()
        } else {
            Redirect::to("/member").into_response()
        };
    }
    render(&state, "login.html", json!({ "error": Value::Null }))
}

pub async fn post_login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let username = clean_string(form.get("username").map(String::as_str).unwrap_or(""), 60);
    let password = form.get("password").cloned().unwrap_or_default();

    if username.is_empty() || password.is_empty() {
        return render(
            &state,
            "login.html",
            json!({ "error": "Please enter username and password." }),
        );
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?1")
        .bind(&username)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

    let Some(user) = user else {
        return render(
            &state,
            "login.html",
            json!({ "error": "Invalid username or password" }),
        );
    };

    let valid = bcrypt::verify(&password, &user.password).unwrap_or(false);
    if !valid {
        return render(
            &state,
            "login.html",
            json!({ "error": "Invalid username or password" }),
        );
    }

    if user.status == "SUSPENDED" {
        return render(
            &state,
            "login.html",
            json!({ "error": "Your account has been suspended. Please contact Admin." }),
        );
    }

    let session = SessionUser {
        id: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
    };
    let token = state.start_session(session).await;

    let dest = if user.role == "ADMIN" {
        "/admin/members"
    } else {
        "/member"
    };
    (jar.add(session_cookie(token)), Redirect::to(dest)).into_response()
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    state.end_session(&jar).await;
    (jar.remove(clear_cookie()), Redirect::to("/login")).into_response()
}

/// Shared change-password handler (scoped to the logged-in session user).
pub async fn change_password(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Value>,
) -> Response {
    let Some(session) = state.current_user(&jar).await else {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "success": false, "message": "Access denied." })),
        )
            .into_response();
    };

    let current_password = body
        .get("current_password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let new_password = body
        .get("new_password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if current_password.is_empty() || new_password.len() < 6 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Current password is required and new password must be at least 6 characters."
            })),
        )
            .into_response();
    }

    let row = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?1")
        .bind(session.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

    let Some(user) = row else {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "success": false, "message": "Access denied." })),
        )
            .into_response();
    };

    if !bcrypt::verify(&current_password, &user.password).unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "message": "Current password is incorrect." })),
        )
            .into_response();
    }

    let new_hash = match bcrypt::hash(&new_password, 10) {
        Ok(h) => h,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": "System error. Please try again." })),
            )
                .into_response()
        }
    };

    if let Err(e) = sqlx::query("UPDATE users SET password = ?1 WHERE id = ?2")
        .bind(new_hash)
        .bind(session.id)
        .execute(&state.db)
        .await
    {
        eprintln!("[ERROR] change-password: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "success": false, "message": "System error. Please try again." })),
        )
            .into_response();
    }

    Json(json!({ "success": true, "message": "Password updated successfully." })).into_response()
}
