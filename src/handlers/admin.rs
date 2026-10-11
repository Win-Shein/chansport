//! Admin Dashboard: matches, results, reports, break lock, member management.

use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::db;
use crate::handlers::{clean_string, json_err, js_num, js_str};
use crate::models::{
    ExposureRow, Match, OutstandingBet, PendingBet, SettledBet, WinLoseAgg,
};
use crate::settlement::{settle_myanmar_bet, BetInput};
use crate::state::{render, AppState, SessionUser};
use crate::timeutil;

async fn require_admin(state: &AppState, jar: &CookieJar) -> Option<SessionUser> {
    let u = state.current_user(jar).await?;
    if u.role == "ADMIN" {
        Some(u)
    } else {
        None
    }
}

fn resolve_filter_date(raw: Option<String>) -> (String, bool) {
    match raw.as_deref() {
        Some("all") => (String::new(), true),
        Some(d) if !d.is_empty() => (d.to_string(), false),
        _ => (timeutil::business_today(), false),
    }
}

fn admin_match_json(m: &Match) -> Value {
    let has_score = m.home_score.is_some() && m.away_score.is_some();
    let is_finished = m.status == "FINISHED" || m.status == "CLOSED";
    let date_disp = timeutil::parse_dt(&m.match_time)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| m.match_time.clone());
    let time_disp = match timeutil::parse_dt(&m.match_time) {
        Some(d) if !matches!(d.format("%H:%M").to_string().as_str(), "00:00" | "12:00") => {
            d.format("%H:%M").to_string()
        }
        _ => String::new(),
    };
    json!({
        "id": m.id,
        "home_team": m.home_team,
        "away_team": m.away_team,
        "upper_team": m.upper_team,
        "body_odd": m.body_odd,
        "total_odd": m.total_odd,
        "status": m.status,
        "home_score": m.home_score,
        "away_score": m.away_score,
        "date_display": date_disp,
        "time_display": time_disp,
        "dateInput": date_disp,
        "timeInput": time_disp,
        "is_finished": is_finished,
        "has_score": has_score,
    })
}

fn result_match_json(m: &Match) -> Value {
    let has_score = m.home_score.is_some() && m.away_score.is_some();
    let date_disp = timeutil::parse_dt(&m.match_time)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| m.match_time.clone());
    let time_disp = match timeutil::parse_dt(&m.match_time) {
        Some(d) if !matches!(d.format("%H:%M").to_string().as_str(), "00:00" | "12:00") => {
            d.format("%H:%M").to_string()
        }
        _ => String::new(),
    };
    json!({
        "id": m.id,
        "home_team": m.home_team,
        "away_team": m.away_team,
        "home_score": m.home_score,
        "away_score": m.away_score,
        "date_display": date_disp,
        "time_display": time_disp,
        "has_score": has_score,
    })
}

// ============================ PAGES ============================

pub async fn add_match_page(State(state): State<AppState>, jar: CookieJar) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };
    render(
        &state,
        "admin_add_match.html",
        json!({ "admin": { "username": admin.username }, "page": "add_match" }),
    )
}

pub async fn match_list(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };
    let (filter_date, show_all) = resolve_filter_date(q.get("filter_date").cloned());
    let date_param: Option<&str> = if filter_date.is_empty() {
        None
    } else {
        Some(&filter_date)
    };

    let matches = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches \
         WHERE (?1 IS NULL OR date(match_time, '-12 hours') = ?1) \
         ORDER BY CASE WHEN status='LIVE' THEN 1 WHEN status='OPEN' THEN 2 ELSE 3 END ASC, match_time ASC",
    )
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let list: Vec<Value> = matches.iter().map(admin_match_json).collect();

    render(
        &state,
        "admin_match_list.html",
        json!({
            "admin": { "username": admin.username },
            "page": "match_list",
            "matches": list,
            "filter_date": filter_date,
            "show_all": show_all,
        }),
    )
}

pub async fn result(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };
    let (filter_date, show_all) = resolve_filter_date(q.get("filter_date").cloned());
    let date_param: Option<&str> = if filter_date.is_empty() {
        None
    } else {
        Some(&filter_date)
    };

    let matches = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches \
         WHERE (?1 IS NULL OR date(match_time, '-12 hours') = ?1) \
         ORDER BY CASE WHEN home_score IS NULL THEN 1 ELSE 2 END ASC, match_time DESC",
    )
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let list: Vec<Value> = matches.iter().map(result_match_json).collect();

    render(
        &state,
        "admin_result.html",
        json!({
            "admin": { "username": admin.username },
            "page": "result",
            "matches": list,
            "filter_date": filter_date,
            "show_all": show_all,
        }),
    )
}

pub async fn report(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };
    let sub_tab = q
        .get("tab")
        .cloned()
        .unwrap_or_else(|| "winlose".to_string());
    let (filter_date, show_all) = resolve_filter_date(q.get("filter_date").cloned());
    let date_param: Option<&str> = if filter_date.is_empty() {
        None
    } else {
        Some(&filter_date)
    };

    // Outstanding bets.
    let outstanding_bets = sqlx::query_as::<_, OutstandingBet>(
        "SELECT b.id, b.user_id, u.username, b.bet_type, b.choice, b.odd, b.amount, b.created_at, \
                m.home_team, m.away_team, m.match_time, m.body_odd, m.total_odd \
         FROM bets b JOIN users u ON b.user_id = u.id JOIN matches m ON b.match_id = m.id \
         WHERE m.status IN ('OPEN','LIVE') AND b.status = 'PENDING' ORDER BY b.created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Settled bets (detail modal).
    let settled_bets = sqlx::query_as::<_, SettledBet>(
        "SELECT b.id, b.user_id, u.username, b.bet_type, b.choice, b.odd, b.amount, b.payout, \
                b.valid_amount, b.status, b.created_at, m.home_team, m.away_team, m.home_score, m.away_score, m.match_time \
         FROM bets b JOIN users u ON b.user_id = u.id JOIN matches m ON b.match_id = m.id \
         WHERE b.status IN ('WON','LOST','DRAW') \
           AND (?1 IS NULL OR date(b.created_at, '-12 hours') = ?1) \
         ORDER BY b.created_at DESC",
    )
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Win/Lose aggregated per member.
    let win_lose = sqlx::query_as::<_, WinLoseAgg>(
        "SELECT u.id as user_id, u.username, COALESCE(u.commission,0.0) as comm_rate, \
                COUNT(b.id) as total_bets, COALESCE(SUM(b.amount),0.0) as turnover, \
                COALESCE(SUM(b.payout),0.0) as payout, COALESCE(SUM(b.valid_amount),0.0) as valid_amount \
         FROM users u \
         LEFT JOIN bets b ON u.id = b.user_id AND b.status IN ('WON','LOST','DRAW') \
              AND (?1 IS NULL OR date(b.created_at, '-12 hours') = ?1) \
         WHERE u.role='MEMBER' GROUP BY u.id, u.username, u.commission ORDER BY turnover DESC",
    )
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut grand_turnover = 0.0;
    let mut grand_valid = 0.0;
    let mut grand_payout = 0.0;
    let mut grand_comm = 0.0;
    let mut grand_member_net = 0.0;

    let win_lose_list: Vec<Value> = win_lose
        .iter()
        .map(|r| {
            let comm_amount = r.valid_amount * r.comm_rate / 100.0;
            let member_net = (r.payout - r.turnover) + comm_amount;
            let net = r.payout - r.turnover;
            grand_turnover += r.turnover;
            grand_valid += r.valid_amount;
            grand_payout += net;
            grand_comm += comm_amount;
            grand_member_net += member_net;
            json!({
                "user_id": r.user_id,
                "username": r.username,
                "total_bets": r.total_bets,
                "turnover": r.turnover,
                "valid_amount": r.valid_amount,
                "payout": r.payout,
                "comm_rate": r.comm_rate,
                "comm_amount": comm_amount,
                "member_net": member_net,
            })
        })
        .collect();

    // Outstanding grouped by member.
    let mut out_map: HashMap<i64, (String, i64, f64)> = HashMap::new();
    for b in &outstanding_bets {
        let e = out_map
            .entry(b.user_id)
            .or_insert_with(|| (b.username.clone(), 0, 0.0));
        e.1 += 1;
        e.2 += b.amount;
    }
    let mut outstanding_by_member: Vec<Value> = out_map
        .into_iter()
        .map(|(user_id, (username, total_bets, total_stake))| {
            json!({
                "user_id": user_id,
                "username": username,
                "total_bets": total_bets,
                "total_stake": total_stake,
            })
        })
        .collect();
    outstanding_by_member.sort_by(|a, b| {
        b["total_stake"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["total_stake"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let outstanding_json: Vec<Value> = outstanding_bets
        .iter()
        .map(|b| {
            json!({
                "user_id": b.user_id,
                "home_team": b.home_team,
                "away_team": b.away_team,
                "choice": b.choice,
                "odd": b.odd,
                "amount": b.amount,
            })
        })
        .collect();

    let settled_json: Vec<Value> = settled_bets
        .iter()
        .map(|b| {
            json!({
                "user_id": b.user_id,
                "home_team": b.home_team,
                "away_team": b.away_team,
                "home_score": b.home_score,
                "away_score": b.away_score,
                "choice": b.choice,
                "odd": b.odd,
                "amount": b.amount,
                "payout": b.payout,
                "status": b.status,
            })
        })
        .collect();

    render(
        &state,
        "admin_report.html",
        json!({
            "admin": { "username": admin.username },
            "page": "report",
            "sub_tab": sub_tab,
            "filter_date": filter_date,
            "show_all": show_all,
            "outstanding_bets": outstanding_json,
            "outstanding_by_member": outstanding_by_member,
            "win_lose_list": win_lose_list,
            "all_settled_bets": settled_json,
            "grand_turnover": grand_turnover,
            "grand_valid": grand_valid,
            "grand_payout": grand_payout,
            "grand_comm": grand_comm,
            "grand_member_net": grand_member_net,
        }),
    )
}

pub async fn break_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };
    let break_limit: f64 = q
        .get("break_limit")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(100000.0);

    let exposure = sqlx::query_as::<_, ExposureRow>(
        "SELECT m.id as match_id, m.home_team, m.away_team, m.match_time, b.bet_type, b.choice, \
                COUNT(b.id) as bet_count, SUM(b.amount) as total_staked \
         FROM bets b JOIN matches m ON b.match_id = m.id \
         WHERE b.status='PENDING' AND m.status IN ('OPEN','LIVE') \
         GROUP BY m.id, m.home_team, m.away_team, m.match_time, b.bet_type, b.choice \
         ORDER BY SUM(b.amount) DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let exposure_list: Vec<Value> = exposure
        .iter()
        .filter(|e| e.total_staked > break_limit)
        .map(|e| {
            json!({
                "home_team": e.home_team,
                "away_team": e.away_team,
                "choice": e.choice,
                "bet_count": e.bet_count,
                "total_staked": e.total_staked,
                "excess": e.total_staked - break_limit,
            })
        })
        .collect();

    let betting_locked = db::is_betting_locked(&state.db).await.unwrap_or(false);

    render(
        &state,
        "admin_break.html",
        json!({
            "admin": { "username": admin.username },
            "page": "break",
            "break_limit": break_limit,
            "exposure_list": exposure_list,
            "betting_locked": betting_locked,
        }),
    )
}

// ============================ ACTIONS ============================

pub async fn toggle_bet_lock(State(state): State<AppState>, jar: CookieJar) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let _ = sqlx::query(
        "UPDATE app_settings SET betting_locked = CASE WHEN betting_locked=1 THEN 0 ELSE 1 END \
         WHERE id = (SELECT id FROM app_settings ORDER BY id LIMIT 1)",
    )
    .execute(&state.db)
    .await;
    Redirect::to("/admin/break").into_response()
}

pub async fn create_member(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Value>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return json_err(StatusCode::FORBIDDEN, "Access denied.");
    }
    let username = clean_string(&js_str(&body, "username"), 50);
    let commission = js_num(&body, "commission");

    if username.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "Username is required.");
    }

    // Members do not log in, so no real password is needed — store a
    // non-verifiable placeholder so the NOT NULL column is satisfied.
    let password_placeholder = format!("!nologin:{}", crate::state::new_token());

    let res = sqlx::query(
        "INSERT INTO users (username, password, role, balance, commission, bet_limit, status) \
         VALUES (?1, ?2, 'MEMBER', 0, ?3, 0, 'ACTIVE')",
    )
    .bind(&username)
    .bind(password_placeholder)
    .bind(commission)
    .execute(&state.db)
    .await;

    match res {
        Ok(_) => Json(json!({ "success": true })).into_response(),
        Err(e) => {
            eprintln!("[ERROR] create-member: {e}");
            json_err(
                StatusCode::BAD_REQUEST,
                "Error creating member. The username may already be taken.",
            )
        }
    }
}

pub async fn edit_member(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let username = clean_string(form.get("username").map(String::as_str).unwrap_or(""), 50);
    let commission = form
        .get("commission")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0);

    if username.is_empty() {
        return (StatusCode::BAD_REQUEST, "Username is required.").into_response();
    }

    let _ = sqlx::query(
        "UPDATE users SET username=?1, commission=?2 WHERE id=?3 AND role='MEMBER'",
    )
    .bind(&username)
    .bind(commission)
    .bind(id)
    .execute(&state.db)
    .await;
    Redirect::to("/home").into_response()
}

pub async fn delete_member(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let _ = sqlx::query("DELETE FROM users WHERE id=?1 AND role='MEMBER'")
        .bind(id)
        .execute(&state.db)
        .await;
    Redirect::to("/home").into_response()
}

pub async fn create_match(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let match_date = clean_string(form.get("match_date").map(String::as_str).unwrap_or(""), 20);
    let home_team = clean_string(form.get("home_team").map(String::as_str).unwrap_or(""), 100);
    let away_team = clean_string(form.get("away_team").map(String::as_str).unwrap_or(""), 100);
    let upper_team = if form.get("upper_team").map(String::as_str) == Some("AWAY") {
        "AWAY"
    } else {
        "HOME"
    };
    let body_odd = clean_string(form.get("body_odd").map(String::as_str).unwrap_or(""), 30);
    let total_odd = clean_string(form.get("total_odd").map(String::as_str).unwrap_or(""), 30);

    if match_date.is_empty() || home_team.is_empty() || away_team.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "Please fill in date and both team names.",
        )
            .into_response();
    }

    let full_timestamp = format!("{} 12:00:00", match_date);
    let _ = sqlx::query(
        "INSERT INTO matches (match_time, home_team, away_team, upper_team, body_odd, total_odd) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&full_timestamp)
    .bind(&home_team)
    .bind(&away_team)
    .bind(upper_team)
    .bind(&body_odd)
    .bind(&total_odd)
    .execute(&state.db)
    .await;
    Redirect::to("/admin/match-list").into_response()
}

pub async fn edit_match(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let match_date = clean_string(form.get("match_date").map(String::as_str).unwrap_or(""), 20);
    let home_team = clean_string(form.get("home_team").map(String::as_str).unwrap_or(""), 100);
    let away_team = clean_string(form.get("away_team").map(String::as_str).unwrap_or(""), 100);
    let upper_team = if form.get("upper_team").map(String::as_str) == Some("AWAY") {
        "AWAY"
    } else {
        "HOME"
    };
    let body_odd = clean_string(form.get("body_odd").map(String::as_str).unwrap_or(""), 30);
    let total_odd = clean_string(form.get("total_odd").map(String::as_str).unwrap_or(""), 30);
    let status = clean_string(form.get("status").map(String::as_str).unwrap_or(""), 20);
    let full_timestamp = format!("{} 12:00:00", match_date);

    let _ = sqlx::query(
        "UPDATE matches SET match_time=?1, home_team=?2, away_team=?3, upper_team=?4, \
                body_odd=?5, total_odd=?6, status=?7 WHERE id=?8",
    )
    .bind(&full_timestamp)
    .bind(&home_team)
    .bind(&away_team)
    .bind(upper_team)
    .bind(&body_odd)
    .bind(&total_odd)
    .bind(&status)
    .bind(id)
    .execute(&state.db)
    .await;
    Redirect::to("/admin/match-list").into_response()
}

pub async fn delete_match(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let _ = sqlx::query("DELETE FROM matches WHERE id=?1")
        .bind(id)
        .execute(&state.db)
        .await;
    Redirect::to("/admin/match-list").into_response()
}

/// ADMIN SET RESULT -> AUTO SETTLE BETS (payout + commission credited to members).
pub async fn set_result(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let home_score = form.get("home_score").and_then(|s| s.trim().parse::<i64>().ok());
    let away_score = form.get("away_score").and_then(|s| s.trim().parse::<i64>().ok());
    let (Some(hs), Some(ags)) = (home_score, away_score) else {
        return (
            StatusCode::BAD_REQUEST,
            "Please enter valid non-negative goal scores.",
        )
            .into_response();
    };
    if hs < 0 || ags < 0 {
        return (
            StatusCode::BAD_REQUEST,
            "Please enter valid non-negative goal scores.",
        )
            .into_response();
    }

    let match_row = sqlx::query_as::<_, Match>("SELECT * FROM matches WHERE id=?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(m) = match_row else {
        return (StatusCode::BAD_REQUEST, "Invalid match id.").into_response();
    };

    if let Err(e) = settle_match(&state.db, &m, hs as i32, ags as i32).await {
        eprintln!("[ERROR] set-result settle: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "System error.").into_response();
    }

    Redirect::to("/admin/result").into_response()
}

/// Set the match score/status and settle all its PENDING bets.
/// Shared by the manual Result form and the automatic result sync.
pub(crate) async fn settle_match(
    db: &sqlx::SqlitePool,
    m: &Match,
    hs: i32,
    ags: i32,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    sqlx::query("UPDATE matches SET home_score=?1, away_score=?2, status='FINISHED' WHERE id=?3")
        .bind(hs)
        .bind(ags)
        .bind(m.id)
        .execute(&mut *tx)
        .await?;

    let pending = sqlx::query_as::<_, PendingBet>(
        "SELECT b.id, b.user_id, b.bet_type, b.choice, b.odd, b.amount, COALESCE(u.commission,0) as comm_rate \
         FROM bets b JOIN users u ON b.user_id = u.id WHERE b.match_id=?1 AND b.status='PENDING'",
    )
    .bind(m.id)
    .fetch_all(&mut *tx)
    .await?;

    for bet in &pending {
        let input = BetInput {
            amount: bet.amount,
            choice: bet.choice.clone(),
            bet_type: bet.bet_type.clone(),
            odd: bet.odd.clone(),
            home_team: m.home_team.clone(),
            away_team: m.away_team.clone(),
            upper_team: m.upper_team.clone(),
        };
        let result = settle_myanmar_bet(&input, hs, ags);
        sqlx::query("UPDATE bets SET status=?1, payout=?2, valid_amount=?3 WHERE id=?4")
            .bind(result.status)
            .bind(result.payout)
            .bind(result.valid_amount)
            .bind(bet.id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

