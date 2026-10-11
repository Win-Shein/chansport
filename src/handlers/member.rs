//! Per-member user dashboard: admin records that member's bets and views history.

use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::db;
use crate::handlers::fixtures::{find_match_for_fav, split_leading_date};
use crate::handlers::{clean_string, js_id, json_err, js_num, js_str};
use crate::models::{Match, RunningBet, User, UserSettledBet};
use crate::settlement::get_opposite_odd;
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
        "home_odd": home_odd,
        "away_odd": away_odd,
        "over_odd": m.total_odd,
        "under_odd": get_opposite_odd(&m.total_odd),
        "is_upper_home": is_home,
        "is_upper_away": !is_home,
        "time_display": time_disp,
    })
}

pub async fn dashboard(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(admin) = require_admin(&state, &jar).await else {
        return Redirect::to("/login").into_response();
    };

    let member = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id=?1 AND role='MEMBER'")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(member) = member else {
        return Redirect::to("/home").into_response();
    };

    let tab = q.get("tab").cloned().unwrap_or_else(|| "bet".to_string());
    let betting_locked = db::is_betting_locked(&state.db).await.unwrap_or(false);

    let matches = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches WHERE status IN ('OPEN','LIVE') ORDER BY match_time ASC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let running_bets = sqlx::query_as::<_, RunningBet>(
        "SELECT b.id, b.choice, b.odd, b.amount, b.created_at, m.home_team, m.away_team \
         FROM bets b JOIN matches m ON b.match_id = m.id \
         WHERE b.user_id=?1 AND b.status='PENDING' ORDER BY b.created_at DESC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let settled_bets = sqlx::query_as::<_, UserSettledBet>(
        "SELECT b.id, b.choice, b.odd, b.amount, b.payout, b.valid_amount, b.status, b.created_at, \
                m.home_team, m.away_team, m.home_score, m.away_score \
         FROM bets b JOIN matches m ON b.match_id = m.id \
         WHERE b.user_id=?1 AND b.status IN ('WON','LOST','DRAW') ORDER BY b.created_at DESC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let comm_rate = member.commission;
    let total_stake: f64 = settled_bets.iter().map(|b| b.amount).sum();
    let total_payout: f64 = settled_bets.iter().map(|b| b.payout).sum();
    let total_valid: f64 = settled_bets.iter().map(|b| b.valid_amount).sum();
    let comm_amount = total_valid * comm_rate / 100.0;
    let net_win_lose = (total_payout - total_stake) + comm_amount;

    let matches_json: Vec<Value> = matches.iter().map(sports_match_json).collect();
    let running_json: Vec<Value> = running_bets
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "home_team": b.home_team,
                "away_team": b.away_team,
                "choice": b.choice,
                "odd": b.odd,
                "amount": b.amount,
                "created_at_display": timeutil::display_dt(&b.created_at),
            })
        })
        .collect();
    let settled_json: Vec<Value> = settled_bets
        .iter()
        .map(|b| {
            let valid = b.valid_amount;
            let comm = valid * comm_rate / 100.0;
            let net = (b.payout - b.amount) + comm;
            json!({
                "id": b.id,
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

    render(
        &state,
        "member_dashboard.html",
        json!({
            "admin": { "username": admin.username },
            "member": {
                "id": member.id,
                "username": member.username,
                "commission": member.commission,
            },
            "tab": tab,
            "betting_locked": betting_locked,
            "matches": matches_json,
            "running_bets": running_json,
            "settled_bets": settled_json,
            "total_stake": total_stake,
            "total_valid": total_valid,
            "comm_amount": comm_amount,
            "net_win_lose": net_win_lose,
        }),
    )
}

pub async fn place_bet(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return json_err(StatusCode::FORBIDDEN, "Access denied.");
    }

    let user_id = if id > 0 { id } else { js_id(&body, "user_id") };
    let match_id = js_id(&body, "match_id");
    let bet_type = js_str(&body, "bet_type");
    let choice = js_str(&body, "choice");
    let bet_amount = js_num(&body, "amount");

    if user_id <= 0 {
        return json_err(StatusCode::BAD_REQUEST, "Member အကောင့် မမှန်ကန်ပါ။");
    }
    if match_id <= 0 {
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

    let member_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE id=?1 AND role='MEMBER'")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if member_exists.is_none() {
        return json_err(StatusCode::BAD_REQUEST, "Member အကောင့် မတွေ့ပါ။");
    }

    let match_row = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches WHERE id=?1 AND status IN ('OPEN','LIVE')",
    )
    .bind(match_id)
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

    if let Err(e) = sqlx::query(
        "INSERT INTO bets (user_id, match_id, bet_type, choice, odd, amount, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'PENDING')",
    )
    .bind(user_id)
    .bind(match_id)
    .bind(&bet_type)
    .bind(&choice)
    .bind(&odd)
    .bind(bet_amount)
    .execute(&state.db)
    .await
    {
        eprintln!("[ERROR] insert bet: {e}");
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, "System error.");
    }

    crate::handlers::json_ok("လောင်းကြေး အောင်မြင်စွာ သွင်းပြီးပါပြီ။")
}

// ============================ PASTE BETS ============================

enum BetKind {
    Body,
    Over,
    Under,
}

struct ParsedBet {
    date: String,
    kind: BetKind,
    team: String,
    amount: f64,
}

/// Selection keyword -> bet kind. Add more synonyms here to support new formats.
///   B / body          -> Body win
///   O / over  / above -> Goals Over (ဂိုးပေါ်)
///   U / under / below -> Goals Under (ဂိုးအောက်)
fn parse_selection(token: &str) -> Option<BetKind> {
    match token.to_ascii_lowercase().as_str() {
        "b" | "body" | "win" | "w" => Some(BetKind::Body),
        "o" | "over" | "ov" | "above" | "up" => Some(BetKind::Over),
        "u" | "under" | "un" | "below" | "low" => Some(BetKind::Under),
        _ => None,
    }
}

/// Parse a bet line. Supported forms (case-insensitive keywords):
///   `Team amount`         -> Body bet
///   `Team B amount`       -> Body bet
///   `Team O amount`       -> Over
///   `Team U amount`       -> Under
///   `Team over/under amount`
///   optional leading `YYYY-MM-DD`
fn parse_bet_line(line: &str, default_date: &str) -> Result<ParsedBet, String> {
    let (text, date_opt) = split_leading_date(line);
    let date = date_opt.unwrap_or_else(|| default_date.to_string());
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 2 {
        return Err("ပုံစံ: Team [B|O|U] amount".to_string());
    }

    let amount_raw = tokens[tokens.len() - 1].replace(',', "");
    let amount: f64 = amount_raw
        .parse()
        .map_err(|_| "ငွေပမာဏ မမှန်ပါ".to_string())?;
    if !(amount > 0.0) {
        return Err("ငွေပမာဏ မမှန်ပါ".to_string());
    }

    // An optional selection keyword sits right before the amount.
    let mut team_end = tokens.len() - 1;
    let mut kind = BetKind::Body;
    if tokens.len() >= 3 {
        if let Some(k) = parse_selection(tokens[tokens.len() - 2]) {
            kind = k;
            team_end = tokens.len() - 2;
        }
    }

    let team = tokens[..team_end].join(" ");
    let team = team.trim().to_string();
    if team.is_empty() {
        return Err("အသင်းနာမည် မပါပါ".to_string());
    }
    Ok(ParsedBet { date, kind, team, amount })
}

/// Edit a running (PENDING) bet's odds and amount.
pub async fn update_bet(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((uid, bid)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let odd = clean_string(form.get("odd").map(String::as_str).unwrap_or(""), 50);
    let amount = form
        .get("amount")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0);

    if !odd.is_empty() && amount > 0.0 {
        let _ = sqlx::query(
            "UPDATE bets SET odd=?1, amount=?2 WHERE id=?3 AND user_id=?4 AND status='PENDING'",
        )
        .bind(&odd)
        .bind(amount)
        .bind(bid)
        .bind(uid)
        .execute(&state.db)
        .await;
    }
    Redirect::to(&format!("/member/{uid}?tab=history")).into_response()
}

/// Delete a bet record (running or settled) for a member.
pub async fn delete_bet(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((uid, bid)): Path<(i64, i64)>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let _ = sqlx::query("DELETE FROM bets WHERE id=?1 AND user_id=?2")
        .bind(bid)
        .bind(uid)
        .execute(&state.db)
        .await;
    Redirect::to(&format!("/member/{uid}?tab=history")).into_response()
}

#[cfg(test)]
mod bet_parse_tests {
    use super::{parse_bet_line, BetKind};

    #[test]
    fn body_with_b() {
        let p = parse_bet_line("Arsenal B 5000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Body));
        assert_eq!(p.team, "Arsenal");
        assert_eq!(p.amount, 5000.0);
    }

    #[test]
    fn body_plain_single_and_multiworld() {
        let p = parse_bet_line("Arsenal 5000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Body));
        assert_eq!(p.team, "Arsenal");
        assert_eq!(p.amount, 5000.0);

        let p = parse_bet_line("Man City 5000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Body));
        assert_eq!(p.team, "Man City");
    }

    #[test]
    fn over_with_o() {
        let p = parse_bet_line("Man City O 10000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Over));
        assert_eq!(p.team, "Man City");
        assert_eq!(p.amount, 10000.0);
    }

    #[test]
    fn under_with_u() {
        let p = parse_bet_line("Chelsea U 3000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Under));
        assert_eq!(p.team, "Chelsea");
    }

    #[test]
    fn word_keywords_and_date() {
        let p = parse_bet_line("2026-12-05 Liverpool over 2000", "2026-01-01").unwrap();
        assert!(matches!(p.kind, BetKind::Over));
        assert_eq!(p.team, "Liverpool");
        assert_eq!(p.date, "2026-12-05");
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn bet_result_page(member_id: i64, added: i32, failed: i32, errors: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Paste Bets - Chan Sport</title>
<script src="https://cdn.tailwindcss.com"></script></head>
<body class="bg-[#eef2f6] text-slate-800 font-sans min-h-screen flex items-center justify-center p-4">
<div class="bg-white rounded-2xl border border-slate-200 shadow-sm max-w-md w-full p-6 space-y-3">
<div class="text-lg font-black text-[#0b2545] text-center">Paste Bets</div>
<div class="flex justify-center gap-2 text-xs">
<span class="px-3 py-1.5 rounded-lg bg-emerald-50 text-emerald-700 font-bold">Added: {added}</span>
<span class="px-3 py-1.5 rounded-lg bg-rose-50 text-rose-700 font-bold">Failed: {failed}</span>
</div>
{errors}
<a href="/member/{member_id}?tab=paste" class="block py-3 bg-[#0b2545] text-amber-300 font-bold rounded-lg text-sm text-center">Back to Paste Bets</a>
<a href="/member/{member_id}?tab=history" class="block py-2.5 bg-slate-200 text-slate-700 font-bold rounded-lg text-xs text-center">View History</a>
</div></body></html>"#
    );
    Html(html).into_response()
}

pub async fn paste_bets(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }

    let member = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id=?1 AND role='MEMBER'")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(member) = member else {
        return Redirect::to("/home").into_response();
    };

    let pasted = form.get("pasted").cloned().unwrap_or_default();
    let default_date = form
        .get("match_date")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(timeutil::business_today);

    if db::is_betting_locked(&state.db).await.unwrap_or(false) {
        return bet_result_page(
            member.id,
            0,
            0,
            "<div class='text-[11px] text-rose-600 bg-rose-50 rounded-lg px-3 py-2 text-center'>လောင်းကြေး ယာယီ ပိတ်ထားပါသည်။</div>",
        );
    }

    let mut added = 0i32;
    let mut failed = 0i32;
    let mut errors = String::new();

    for raw_line in pasted.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let pb = match parse_bet_line(line, &default_date) {
            Ok(p) => p,
            Err(e) => {
                failed += 1;
                errors.push_str(&format!(
                    "<div class='text-[11px] text-rose-600 bg-rose-50 rounded-lg px-3 py-1.5 break-all'>{}</div>",
                    escape(&format!("{line} — {e}"))
                ));
                continue;
            }
        };

        let Some((match_id, is_home)) = find_match_for_fav(&state, &pb.date, &pb.team).await else {
            failed += 1;
            errors.push_str(&format!(
                "<div class='text-[11px] text-rose-600 bg-rose-50 rounded-lg px-3 py-1.5 break-all'>{}</div>",
                escape(&format!("{line} — '{}' နှင့် ကိုက်ညီသောပွဲ မတွေ့ပါ ({})", pb.team, pb.date))
            ));
            continue;
        };

        let m = sqlx::query_as::<_, Match>("SELECT * FROM matches WHERE id=?1")
            .bind(match_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
        let Some(m) = m else {
            failed += 1;
            continue;
        };

        let is_upper_home = m.upper_team == "HOME";
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

        let (bet_type, choice, odd) = match pb.kind {
            BetKind::Body => {
                if is_home {
                    ("BODY", m.home_team.clone(), home_odd)
                } else {
                    ("BODY", m.away_team.clone(), away_odd)
                }
            }
            BetKind::Over => ("OVER_UNDER", "OVER (Over)".to_string(), m.total_odd.clone()),
            BetKind::Under => (
                "OVER_UNDER",
                "UNDER (Under)".to_string(),
                get_opposite_odd(&m.total_odd),
            ),
        };

        let res = sqlx::query(
            "INSERT INTO bets (user_id, match_id, bet_type, choice, odd, amount, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'PENDING')",
        )
        .bind(member.id)
        .bind(match_id)
        .bind(bet_type)
        .bind(&choice)
        .bind(&odd)
        .bind(pb.amount)
        .execute(&state.db)
        .await;

        if res.is_ok() {
            added += 1;
        } else {
            failed += 1;
        }
    }

    bet_result_page(member.id, added, failed, &errors)
}
