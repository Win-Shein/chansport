//! SQLite row structs used with sqlx `query_as`.

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password: String,
    pub role: String,
    pub balance: f64,
    pub commission: f64,
    pub bet_limit: f64,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Member {
    pub id: i64,
    pub username: String,
    pub balance: f64,
    pub commission: f64,
    pub bet_limit: f64,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Match {
    pub id: i64,
    pub match_time: String,
    pub home_team: String,
    pub away_team: String,
    pub upper_team: String,
    pub body_odd: String,
    pub total_odd: String,
    pub home_score: Option<i64>,
    pub away_score: Option<i64>,
    pub status: String,
    pub created_at: String,
}

/// A running (PENDING) bet joined with its match — used on the member dashboard.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RunningBet {
    pub id: i64,
    pub choice: String,
    pub odd: String,
    pub amount: f64,
    pub created_at: String,
    pub home_team: String,
    pub away_team: String,
}

/// A settled bet joined with its match — used on the member "Win/Lose" tab.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserSettledBet {
    pub id: i64,
    pub choice: String,
    pub odd: String,
    pub amount: f64,
    pub payout: f64,
    pub valid_amount: f64,
    pub status: String,
    pub home_team: String,
    pub away_team: String,
    pub home_score: Option<i64>,
    pub away_score: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreditLog {
    pub id: i64,
    pub user_id: i64,
    pub log_type: String,
    pub amount: f64,
    pub balance_after: f64,
    pub note: Option<String>,
    pub admin_username: Option<String>,
    pub created_at: String,
}

/// Credit log joined with the owning member's username.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreditLogWithUser {
    pub id: i64,
    pub user_id: i64,
    pub log_type: String,
    pub amount: f64,
    pub balance_after: f64,
    pub note: Option<String>,
    pub admin_username: Option<String>,
    pub created_at: String,
    pub username: String,
}

/// Outstanding (PENDING) bet on an open/live match, with owner + match info.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutstandingBet {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub bet_type: String,
    pub choice: String,
    pub odd: String,
    pub amount: f64,
    pub created_at: String,
    pub home_team: String,
    pub away_team: String,
    pub match_time: String,
    pub body_odd: String,
    pub total_odd: String,
}

/// Settled bet with owner + match info (admin report detail modal).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SettledBet {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub bet_type: String,
    pub choice: String,
    pub odd: String,
    pub amount: f64,
    pub payout: f64,
    pub valid_amount: f64,
    pub status: String,
    pub created_at: String,
    pub home_team: String,
    pub away_team: String,
    pub home_score: Option<i64>,
    pub away_score: Option<i64>,
    pub match_time: String,
}

/// Per-member aggregated Win/Lose numbers.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WinLoseAgg {
    pub user_id: i64,
    pub username: String,
    pub comm_rate: f64,
    pub total_bets: i64,
    pub turnover: f64,
    pub payout: f64,
    pub valid_amount: f64,
}

/// Combined exposure per match + selection (break monitor).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExposureRow {
    pub match_id: i64,
    pub home_team: String,
    pub away_team: String,
    pub match_time: String,
    pub bet_type: String,
    pub choice: String,
    pub bet_count: i64,
    pub total_staked: f64,
}

/// PENDING bet on a match being settled, with the owner's commission rate.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingBet {
    pub id: i64,
    pub user_id: i64,
    pub bet_type: String,
    pub choice: String,
    pub odd: String,
    pub amount: f64,
    pub comm_rate: f64,
}
