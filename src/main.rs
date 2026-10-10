//! MYANBET365 — Myanmar football odds / betting management system.
//!
//! Rust + Axum + SQLx(SQLite) port of the original Node.js/PostgreSQL app.

mod db;
mod handlers;
mod models;
mod settlement;
mod state;
mod timeutil;

use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use state::{AppState, SessionUser};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env if present (optional).
    dotenvy::dotenv().ok();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://myanbet365.db".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3001".to_string());

    // Ensure the SQLite parent directory exists (for "sqlite://data/app.db" style URLs).
    if let Some(path) = database_url.strip_prefix("sqlite://") {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
    }

    let pool = db::init_pool(&database_url).await?;
    db::migrate(&pool).await?;
    db::seed(&pool).await?;

    let templates_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    let env = state::build_env(&templates_dir)?;

    let app_state = AppState {
        db: pool,
        sessions: Arc::new(RwLock::new(HashMap::<String, SessionUser>::new())),
        env: Arc::new(env),
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/login", get(handlers::auth::get_login).post(handlers::auth::post_login))
        .route("/logout", get(handlers::auth::logout))
        .route("/admin", get(admin_root))
        .route("/admin/change-password", post(handlers::auth::change_password))
        .route("/member/change-password", post(handlers::auth::change_password))
        .route("/member", get(handlers::member::dashboard))
        .route("/member/bet", post(handlers::member::place_bet))
        .route("/admin/members", get(handlers::admin::members))
        .route(
            "/admin/add-match",
            get(handlers::admin::add_match_page).post(handlers::admin::create_match),
        )
        .route("/admin/match-list", get(handlers::admin::match_list))
        .route("/admin/result", get(handlers::admin::result))
        .route("/admin/report", get(handlers::admin::report))
        .route("/admin/break", get(handlers::admin::break_page))
        .route("/admin/toggle-bet-lock", post(handlers::admin::toggle_bet_lock))
        .route("/admin/create-member", post(handlers::admin::create_member))
        .route("/admin/edit-member/{id}", post(handlers::admin::edit_member))
        .route("/admin/deposit-member/{id}", post(handlers::admin::deposit_member))
        .route("/admin/withdraw-member/{id}", post(handlers::admin::withdraw_member))
        .route(
            "/admin/toggle-status-member/{id}",
            post(handlers::admin::toggle_status_member),
        )
        .route("/admin/delete-member/{id}", post(handlers::admin::delete_member))
        .route("/admin/edit-match/{id}", post(handlers::admin::edit_match))
        .route("/admin/delete-match/{id}", post(handlers::admin::delete_match))
        .route("/admin/set-result/{id}", post(handlers::admin::set_result))
        .with_state(app_state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("🚀 Server running on http://localhost:{port}");
    println!("🔑 Admin  : username = admin | password = admin123");
    println!("👤 Member : username = win   | password = 123456");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn root() -> axum::response::Redirect {
    axum::response::Redirect::to("/login")
}

async fn admin_root() -> axum::response::Redirect {
    axum::response::Redirect::to("/admin/members")
}
