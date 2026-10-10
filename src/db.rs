//! SQLite connection pool, schema migration and seeding.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

const SCHEMA: &str = include_str!("../schema.sql");

pub async fn init_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for stmt in SCHEMA.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            sqlx::query(s).execute(pool).await?;
        }
    }
    Ok(())
}

/// Seed the default admin + demo member accounts (bcrypt hashed), if missing.
pub async fn seed(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    let admin_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = 'admin'")
        .fetch_one(pool)
        .await?;
    if admin_exists == 0 {
        let hash = bcrypt::hash("admin123", 10)?;
        sqlx::query(
            "INSERT INTO users (username, password, role, balance, commission, bet_limit, status) \
             VALUES ('admin', ?1, 'ADMIN', 0, 0, 0, 'ACTIVE')",
        )
        .bind(hash)
        .execute(pool)
        .await?;
    }

    let win_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = 'win'")
        .fetch_one(pool)
        .await?;
    if win_exists == 0 {
        let hash = bcrypt::hash("123456", 10)?;
        sqlx::query(
            "INSERT INTO users (username, password, role, balance, commission, bet_limit, status) \
             VALUES ('win', ?1, 'MEMBER', 100000, 5, 50000, 'ACTIVE')",
        )
        .bind(hash)
        .execute(pool)
        .await?;
    }

    let settings_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(pool)
        .await?;
    if settings_exists == 0 {
        sqlx::query("INSERT INTO app_settings (betting_locked) VALUES (0)")
            .execute(pool)
            .await?;
    }

    Ok(())
}

/// Global bet lock flag.
pub async fn is_betting_locked(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let locked: Option<i64> =
        sqlx::query_scalar("SELECT betting_locked FROM app_settings ORDER BY id LIMIT 1")
            .fetch_optional(pool)
            .await?;
    Ok(locked.unwrap_or(0) != 0)
}
