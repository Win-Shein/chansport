//! Member dashboard + bet placement handlers.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::db;
use crate::handlers::{is_valid_id, json_err, js_num, js_str};
use crate::models::{CreditLog, Match, RunningBet, User, UserSettledBet};
use crate::settlement::get_opposite_odd;
use crate::state::{render, AppState};
use crate::timeutil;

fn resolve_filter_date(raw: Option<String>) -> (String, bool) {
    match raw.as_deref() {
        Some("all") => (String::new(), true),
        Some(d) if !d.is_empty() => (d.to_string(), false),
        _ => (timeutil::business_today(), false),
    }
}

fn time_only(dt: &str) -> String {
    timeutil::parse_dt(dt)
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_else(|| dt.to_string())
}

fn date_display(dt: &str) -> String {
    timeutil::parse_dt(dt)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| dt.to_string())
}

fn date_input(dt: &str) -> String {
    timeutil::parse_dt(dt)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| dt.to_string())
}

fn time_input(dt: &str) -> String {
    timeutil::parse_dt(dt)
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn sports_match_json(m: &Match) -> Value {
    let is_home = m.upper_team == "HOME";
    let home_odd = if is_home {
        m.body_odd.clone()
    } else {
        get_opposite_odd(&m.body_odd)
    };
    let away_odd = if is_home {
        get_opposite_odd(&m.body_odd)
    } else {
        m.body_odd.clone()
    };
    json!({
        "id": m.id,
        "home_team": m.home_team,
        "away_team": m.away_team,
        "upper_team": m.upper_team,
        "body_odd": m.body_odd,
        "total_odd": m.total_odd,
        "home_odd": home_odd,
        "away_odd": away_odd,
        "over_odd": m.total_odd,
        "under_odd": get_opposite_odd(&m.total_odd),
        "is_upper_home": is_home,
        "is_upper_away": !is_home,
        "time_display": time_only(&m.match_time),
    })
}

pub async fn dashboard(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(session) = state.current_user(&jar).await else {
        return Redirect::to("/login").into_response();
    };

    let current_tab = q.get("tab").cloned().unwrap_or_else(|| "sports".to_string());
    let (filter_date, show_all) = resolve_filter_date(q.get("filter_date").cloned());

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?1")
        .bind(session.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(user) = user else {
        return Redirect::to("/login").into_response();
    };

    let betting_locked = db::is_betting_locked(&state.db).await.unwrap_or(false);
    let now = timeutil::now_str();
    let date_param: Option<&str> = if filter_date.is_empty() {
        None
    } else {
        Some(&filter_date)
    };

    // Active open/live matches whose kickoff is still in the future.
    let matches = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches WHERE status IN ('OPEN','LIVE') AND match_time > ?1 ORDER BY match_time ASC",
    )
    .bind(&now)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Running (PENDING) bets.
    let running_bets = sqlx::query_as::<_, RunningBet>(
        "SELECT b.id, b.choice, b.odd, b.amount, b.created_at, m.home_team, m.away_team \
         FROM bets b JOIN matches m ON b.match_id = m.id \
         WHERE b.user_id = ?1 AND b.status = 'PENDING' ORDER BY b.created_at DESC",
    )
    .bind(session.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Settled bets (WON/LOST/DRAW).
    let win_lose_bets = sqlx::query_as::<_, UserSettledBet>(
        "SELECT b.id, b.choice, b.odd, b.amount, b.payout, b.valid_amount, b.status, b.created_at, \
                m.home_team, m.away_team, m.home_score, m.away_score \
         FROM bets b JOIN matches m ON b.match_id = m.id \
         WHERE b.user_id = ?1 AND b.status IN ('WON','LOST','DRAW') \
           AND (?2 IS NULL OR date(b.created_at, '+12 hours') = ?2) \
         ORDER BY b.created_at DESC",
    )
    .bind(session.id)
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Deposit / withdraw history.
    let credit_log = sqlx::query_as::<_, CreditLog>(
        "SELECT id, user_id, type AS log_type, amount, balance_after, note, admin_username, created_at \
         FROM credit_logs WHERE user_id = ?1 \
           AND (?2 IS NULL OR date(created_at, '+12 hours') = ?2) ORDER BY created_at DESC",
    )
    .bind(session.id)
    .bind(date_param)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let comm_rate = user.commission;
    let total_stake: f64 = win_lose_bets.iter().map(|r| r.amount).sum();
    let total_payout: f64 = win_lose_bets.iter().map(|r| r.payout).sum();
    let total_valid_amount: f64 = win_lose_bets.iter().map(|r| r.valid_amount).sum();
    let comm_amount = total_valid_amount * comm_rate / 100.0;
    let net_win_lose = (total_payout - total_stake) + comm_amount;

    let matches_json: Vec<Value> = matches.iter().map(sports_match_json).collect();
    let running_json: Vec<Value> = running_bets
        .iter()
        .map(|b| {
            json!({
                "home_team": b.home_team,
                "away_team": b.away_team,
                "choice": b.choice,
                "odd": b.odd,
                "amount": b.amount,
                "created_at_display": timeutil::display_dt(&b.created_at),
            })
        })
        .collect();
    let winlose_json: Vec<Value> = win_lose_bets
        .iter()
        .map(|b| {
            let valid = b.valid_amount;
            let comm = valid * comm_rate / 100.0;
            let net = (b.payout - b.amount) + comm;
            json!({
                "home_team": b.home_team,
                "away_team": b.away_team,
                "choice": b.choice,
                "odd": b.odd,
                "home_score": b.home_score,
                "away_score": b.away_score,
                "amount": b.amount,
                "valid": valid,
                "comm": comm,
                "net": net,
                "status": b.status,
            })
        })
        .collect();
    let credit_json: Vec<Value> = credit_log
        .iter()
        .map(|l| {
            json!({
                "log_type": l.log_type,
                "amount": l.amount,
                "balance_after": l.balance_after,
                "note": l.note,
                "created_at_display": timeutil::display_dt(&l.created_at),
            })
        })
        .collect();

    render(
        &state,
        "member_dashboard.html",
        json!({
            "user": {
                "username": user.username,
                "balance": user.balance,
                "bet_limit": user.bet_limit,
            },
            "current_tab": current_tab,
            "filter_date": filter_date,
            "show_all": show_all,
            "betting_locked": betting_locked,
            "matches": matches_json,
            "running_bets": running_json,
            "win_lose_bets": winlose_json,
            "credit_log": credit_json,
            "comm_rate": comm_rate,
            "total_stake": total_stake,
            "total_valid_amount": total_valid_amount,
            "comm_amount": comm_amount,
            "net_win_lose": net_win_lose,
        }),
    )
}

pub async fn place_bet(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Value>,
) -> Response {
    let Some(session) = state.current_user(&jar).await else {
        return json_err(StatusCode::FORBIDDEN, "Access denied.");
    };

    let match_id = js_str(&body, "match_id");
    let bet_type = js_str(&body, "bet_type");
    let choice = js_str(&body, "choice");
    let bet_amount = js_num(&body, "amount");

    if !is_valid_id(&match_id) {
        return json_err(StatusCode::BAD_REQUEST, "လောင်းကြေးတင်မည့် ပွဲစဉ် မမှန်ကန်ပါ။");
    }
    if bet_type != "BODY" && bet_type != "OVER_UNDER" {
        return json_err(StatusCode::BAD_REQUEST, "လောင်းကြေး အမျိုးအစား မမှန်ကန်ပါ။");
    }
    if !(bet_amount > 0.0) {
        return json_err(StatusCode::BAD_REQUEST, "လောင်းကြေး ပမာဏ မှားယွင်းနေပါသည်။");
    }

    if db::is_betting_locked(&state.db).await.unwrap_or(false) {
        return json_err(
            StatusCode::FORBIDDEN,
            "စနစ်တွင် လောင်းကြေး ယာယီ ပိတ်ထားပါသည်။ ခဏစောင့်ပြီး ပြန်ကြိုးစားပါ။",
        );
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?1")
        .bind(session.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(user) = user else {
        return json_err(StatusCode::FORBIDDEN, "Access denied.");
    };
    if user.status == "SUSPENDED" {
        return json_err(
            StatusCode::FORBIDDEN,
            "သင့်အကောင့်ကို ပိတ်ထားပါသဖြင့် လောင်း၍မရနိုင်ပါ။",
        );
    }

    let match_row = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches WHERE id = ?1 AND status IN ('OPEN','LIVE')",
    )
    .bind(match_id.parse::<i64>().unwrap_or(0))
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(m) = match_row else {
        return json_err(
            StatusCode::BAD_REQUEST,
            "ဒီပွဲစဉ်သည် လောင်းကြေး လက်ခံခြင်း မရှိတော့ပါ။",
        );
    };

    // Close betting 5 minutes before kickoff.
    if let Some(kickoff) = timeutil::parse_dt(&m.match_time) {
        if kickoff <= timeutil::now_myanmar() + chrono::Duration::minutes(5) {
            return json_err(
                StatusCode::BAD_REQUEST,
                "ပွဲစဉ် မစခင် ၅ မိနစ် အတွင်း လောင်းကြေး လက်ခံခြင်း မရှိတော့ပါ။",
            );
        }
    }

    // Derive authoritative odd from the match (never trust the client).
    let is_upper_home = m.upper_team == "HOME";
    let odd = if bet_type == "BODY" {
        let home_odd = if is_upper_home {
            m.body_odd.clone()
        } else {
            get_opposite_odd(&m.body_odd)
        };
        let away_odd = if is_upper_home {
            get_opposite_odd(&m.body_odd)
        } else {
            m.body_odd.clone()
        };
        if choice == m.home_team {
            Some(home_odd)
        } else if choice == m.away_team {
            Some(away_odd)
        } else {
            None
        }
    } else {
        let over_odd = m.total_odd.clone();
        let under_odd = get_opposite_odd(&m.total_odd);
        if choice == "OVER (Over)" {
            Some(over_odd)
        } else if choice == "UNDER (Under)" {
            Some(under_odd)
        } else {
            None
        }
    };
    let Some(odd) = odd else {
        return json_err(StatusCode::BAD_REQUEST, "ရွေးချယ်မှု မမှန်ကန်ပါ။");
    };

    if bet_amount > user.bet_limit {
        return json_err(
            StatusCode::BAD_REQUEST,
            &format!(
                "အများဆုံး လောင်းနိုင်သော ပမာဏမှာ {} MMK ဖြစ်ပါသည်!",
                timeutil::money(user.bet_limit)
            ),
        );
    }
    if user.balance < bet_amount {
        return json_err(
            StatusCode::BAD_REQUEST,
            "လက်ကျန်ငွေ မလုံလောက်ပါ။ ကျေးဇူးပြု၍ ငွေဖြည့်သွင်းပါ။",
        );
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("[ERROR] begin tx: {e}");
            return json_err(StatusCode::INTERNAL_SERVER_ERROR, "System error. Please contact support.");
        }
    };

    if let Err(e) = sqlx::query("UPDATE users SET balance = balance - ?1 WHERE id = ?2")
        .bind(bet_amount)
        .bind(session.id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("[ERROR] deduct balance: {e}");
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "System error. Please contact support.");
    }
    if let Err(e) = sqlx::query(
        "INSERT INTO bets (user_id, match_id, bet_type, choice, odd, amount, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'PENDING')",
    )
    .bind(session.id)
    .bind(match_id.parse::<i64>().unwrap_or(0))
    .bind(&bet_type)
    .bind(&choice)
    .bind(&odd)
    .bind(bet_amount)
    .execute(&mut *tx)
    .await
    {
        eprintln!("[ERROR] insert bet: {e}");
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "System error. Please contact support.");
    }
    if let Err(e) = tx.commit().await {
        eprintln!("[ERROR] commit bet: {e}");
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "System error. Please contact support.");
    }

    crate::handlers::json_ok("လောင်းကြေး အောင်မြင်စွာ တင်ပြီးပါပြီ။")
}
