//! Shared application state, template environment and in-memory sessions.

use axum::response::{Html, IntoResponse, Response};
use axum::http::StatusCode;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use minijinja::Environment;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use sqlx::SqlitePool;

pub const SESSION_COOKIE: &str = "myanbet.sid";

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub sessions: Arc<RwLock<HashMap<String, SessionUser>>>,
    pub env: Arc<Environment<'static>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: String,
}

impl AppState {
    pub async fn current_user(&self, jar: &CookieJar) -> Option<SessionUser> {
        let token = jar.get(SESSION_COOKIE)?.value().to_string();
        let map = self.sessions.read().await;
        map.get(&token).cloned()
    }

    pub async fn start_session(&self, user: SessionUser) -> String {
        let token = new_token();
        self.sessions.write().await.insert(token.clone(), user);
        token
    }

    pub async fn end_session(&self, jar: &CookieJar) {
        if let Some(c) = jar.get(SESSION_COOKIE) {
            let token = c.value().to_string();
            self.sessions.write().await.remove(&token);
        }
    }
}

pub fn new_token() -> String {
    let a: u128 = rand::random();
    let b: u128 = rand::random();
    format!("{a:032x}{b:032x}")
}

pub fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

pub fn clear_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

/// Render a template with the given context. Errors are logged, never leaked.
pub fn render(state: &AppState, name: &str, ctx: impl Serialize) -> Response {
    let result = match state.env.get_template(name) {
        Ok(t) => {
            let value = minijinja::Value::from(minijinja::value::Serde(&ctx));
            t.render(value)
        }
        Err(e) => Err(e),
    };

    match result {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err(e) => {
            eprintln!("[template] {name}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Please try again.",
            )
                .into_response()
        }
    }
}

fn json_filter(value: minijinja::Value) -> Result<minijinja::Value, minijinja::Error> {
    let s = serde_json::to_string(&value).map_err(|e| {
        minijinja::Error::new(
            minijinja::ErrorKind::InvalidOperation,
            format!("json filter: {e}"),
        )
    })?;
    Ok(minijinja::Value::from_safe_string(s))
}

fn money_filter(v: f64) -> String {
    crate::timeutil::money(v)
}

/// Build the minijinja environment and load every template under `templates/`.
pub fn build_env(templates_dir: &Path) -> Result<Environment<'static>, minijinja::Error> {
    let mut env = Environment::new();
    env.add_filter("money", money_filter);
    env.add_filter("json", json_filter);
    load_templates(&mut env, templates_dir, templates_dir)?;
    Ok(env)
}

fn load_templates(
    env: &mut Environment<'static>,
    dir: &Path,
    base: &Path,
) -> Result<(), minijinja::Error> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))?;
    for entry in entries {
        let entry = entry
            .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            load_templates(env, &path, base)?;
        } else {
            let name = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let src = std::fs::read_to_string(&path).map_err(|e| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
            })?;
            env.add_template_owned(name, src)?;
        }
    }
    Ok(())
}
