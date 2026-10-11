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

/// Delete matches (and their bets via ON DELETE CASCADE) whose matchday is
/// older than `days`. Returns the number of matches removed.
pub async fn cleanup_old_records(pool: &SqlitePool, days: i64) -> Result<u64, sqlx::Error> {
    let cutoff = crate::timeutil::business_today_minus_days(days);
    let result = sqlx::query("DELETE FROM matches WHERE date(match_time, '-12 hours') < ?1")
        .bind(&cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Background cleanup: keep only the last `DATA_RETENTION_DAYS` (default 14)
/// days of records, deleting older ones. Set DATA_RETENTION_DAYS=0 to disable.
pub fn spawn_retention_cleanup(pool: SqlitePool) {
    let days: i64 = std::env::var("DATA_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(14);
    if days <= 0 {
        return;
    }
    tokio::spawn(async move {
        loop {
            match cleanup_old_records(&pool, days).await {
                Ok(0) => {}
                Ok(n) => println!("[retention] removed {n} old match(es) (kept {days} days)"),
                Err(e) => eprintln!("[retention] error: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

/// Global bet lock flag.
pub async fn is_betting_locked(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let locked: Option<i64> =
        sqlx::query_scalar("SELECT betting_locked FROM app_settings ORDER BY id LIMIT 1")
            .fetch_optional(pool)
            .await?;
    Ok(locked.unwrap_or(0) != 0)
}
