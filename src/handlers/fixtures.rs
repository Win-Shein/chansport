//! Football fixtures: API sync (with league filter) and paste-and-parse odds.
//!
//! Paste supports two line forms:
//!   1. `FavTeam bodyOdd/totalOdd`  -> find the matching fixture by team name
//!      (fuzzy) and set that team as Upper + the odds.
//!   2. `Home vs Away FavTeam bodyOdd/totalOdd` -> insert a brand new match.
//! A leading `YYYY-MM-DD` on either form overrides the chosen date.

use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

use crate::models::Match;
use crate::state::{AppState, SessionUser};
use crate::timeutil;

/// Minimum fuzzy similarity between the pasted Fav name and a team name.
const FAV_MATCH_THRESHOLD: f64 = 0.6;

async fn require_admin(state: &AppState, jar: &CookieJar) -> Option<SessionUser> {
    let u = state.current_user(jar).await?;
    if u.role == "ADMIN" {
        Some(u)
    } else {
        None
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn result_page(
    title: &str,
    message: &str,
    added: i32,
    updated: i32,
    skipped: i32,
    failed: i32,
    errors: &str,
) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title} - Chan Sport</title>
<script src="https://cdn.tailwindcss.com"></script></head>
<body class="bg-[#eef2f6] text-slate-800 font-sans min-h-screen flex items-center justify-center p-4">
<div class="bg-white rounded-2xl border border-slate-200 shadow-sm max-w-md w-full p-6 space-y-3">
<div class="text-lg font-black text-[#0b2545] text-center">{title}</div>
<div class="text-sm text-slate-600 text-center">{message}</div>
<div class="grid grid-cols-4 gap-2 text-center text-xs">
<div class="px-2 py-1.5 rounded-lg bg-emerald-50 text-emerald-700 font-bold">Added<br>{added}</div>
<div class="px-2 py-1.5 rounded-lg bg-blue-50 text-blue-700 font-bold">Updated<br>{updated}</div>
<div class="px-2 py-1.5 rounded-lg bg-slate-100 text-slate-600 font-bold">Skipped<br>{skipped}</div>
<div class="px-2 py-1.5 rounded-lg bg-rose-50 text-rose-700 font-bold">Failed<br>{failed}</div>
</div>
{errors}
<a href="/admin/match-list" class="block py-3 bg-[#0b2545] text-amber-300 font-bold rounded-lg text-sm text-center">Go to Match List</a>
<a href="/admin/add-match" class="block py-2.5 bg-slate-200 text-slate-700 font-bold rounded-lg text-xs text-center">Back to Add Match</a>
</div></body></html>"#
    );
    Html(html).into_response()
}

// ============================ API SYNC ============================

/// Default provider: TheSportsDB free endpoint. Override via FIXTURES_API_URL.
const DEFAULT_FIXTURES_URL: &str =
    "https://www.thesportsdb.com/api/v1/json/3/eventsday.php?d={date}&s=Soccer";

struct FixtureEvent {
    home: String,
    away: String,
    date: String,
    time: String,
    league: String,
    league_id: String,
    home_score: Option<i64>,
    away_score: Option<i64>,
}

fn json_path_i64(value: &Value, path: &str) -> Option<i64> {
    let mut cur = value;
    for part in path.split('.') {
        cur = cur.get(part)?;
    }
    if let Some(n) = cur.as_i64() {
        return Some(n);
    }
    cur.as_str().and_then(|s| s.trim().parse::<i64>().ok())
}

fn first_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(n) = json_path_i64(value, k) {
            return Some(n);
        }
    }
    None
}

fn json_path_str(value: &Value, path: &str) -> String {
    let mut cur = value;
    for part in path.split('.') {
        match cur.get(part) {
            Some(next) => cur = next,
            None => return String::new(),
        }
    }
    cur.as_str().map(|s| s.trim().to_string()).unwrap_or_default()
}

fn first_str(value: &Value, keys: &[&str]) -> String {
    for k in keys {
        let s = json_path_str(value, k);
        if !s.is_empty() {
            return s;
        }
    }
    String::new()
}

fn fetch_fixtures(url: &str) -> Result<Vec<FixtureEvent>, String> {
    let mut request = ureq::get(url).timeout(Duration::from_secs(25));
    // Optional API key sent as a header (used by API-Football / api-sports.io).
    if let Ok(key) = std::env::var("FIXTURES_API_KEY") {
        let key = key.trim();
        if !key.is_empty() {
            request = request.set("x-apisports-key", key);
        }
    }
    let response = request.call().map_err(|e| e.to_string())?;
    let value: Value = response.into_json().map_err(|e| e.to_string())?;

    let array = value
        .get("events")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("response").and_then(|v| v.as_array()))
        .or_else(|| value.as_array());

    let mut out = Vec::new();
    if let Some(items) = array {
        for item in items {
            let home = first_str(item, &["strHomeTeam", "home_team", "teams.home.name", "home.name"]);
            let away = first_str(item, &["strAwayTeam", "away_team", "teams.away.name", "away.name"]);
            let date = first_str(item, &["dateEvent", "strDate", "date", "fixture.date"]);
            let time = first_str(item, &["strTime", "time", "fixture.time", "match_time"]);
            let league = first_str(item, &["strLeague", "strLeagueName", "league.name", "league"]);
            let league_id = first_str(item, &["idLeague", "league.id"]);
            let home_score = first_i64(item, &["intHomeScore", "home_score", "goals.home"]);
            let away_score = first_i64(item, &["intAwayScore", "away_score", "goals.away"]);
            out.push(FixtureEvent {
                home,
                away,
                date,
                time,
                league,
                league_id,
                home_score,
                away_score,
            });
        }
    }
    Ok(out)
}

/// Leagues configured via FIXTURES_LEAGUES (comma separated). Empty = all.
fn configured_leagues() -> Vec<String> {
    std::env::var("FIXTURES_LEAGUES")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// League ids configured via FIXTURES_LEAGUE_IDS (exact match, e.g. API-Football).
fn configured_league_ids() -> Vec<String> {
    std::env::var("FIXTURES_LEAGUE_IDS")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// A match is allowed if its league id is listed, or (fallback) its league name
/// contains one of the configured name substrings.
fn league_allowed(name: &str, id: &str, names: &[String], ids: &[String]) -> bool {
    if !ids.is_empty() && ids.iter().any(|i| i == id) {
        return true;
    }
    let n = name.to_lowercase();
    names.iter().any(|a| n.contains(a.as_str()))
}

fn next_date(date: &str) -> Option<String> {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|d| (d + chrono::Duration::days(1)).format("%Y-%m-%d").to_string())
}

pub async fn sync_fixtures(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }

    // Only import the configured leagues — never everything by accident.
    let allowed = configured_leagues();
    let allowed_ids = configured_league_ids();
    if allowed.is_empty() && allowed_ids.is_empty() {
        return result_page(
            "Fixture Sync",
            "FIXTURES_LEAGUES (သို့ FIXTURES_LEAGUE_IDS) မသတ်မှတ်ရသေးပါ — import လိုသော league စာရင်းကို .env တွင် ထည့်ပါ။",
            0,
            0,
            0,
            0,
            "",
        );
    }

    let requested_date = form
        .get("sync_date")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(timeutil::business_today);

    let url_tmpl = std::env::var("FIXTURES_API_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FIXTURES_URL.to_string());

    // Fetch the chosen matchday AND the next calendar day, then keep only the
    // events that belong to the chosen matchday (evening of X + early morning
    // of X+1), using the same noon-to-noon cutoff as the rest of the app.
    let mut dates = vec![requested_date.clone()];
    if let Some(n) = next_date(&requested_date) {
        dates.push(n);
    }

    let mut events: Vec<FixtureEvent> = Vec::new();
    for (idx, d) in dates.iter().enumerate() {
        let url = url_tmpl.replace("{date}", d);
        match tokio::task::spawn_blocking(move || fetch_fixtures(&url)).await {
            Ok(Ok(list)) => events.extend(list),
            Ok(Err(e)) => {
                if idx == 0 {
                    return result_page(
                        "Fixture Sync",
                        "API မှ ဆွဲယူ၍ မရပါ။ FIXTURES_API_URL ကို စစ်ပါ။",
                        0,
                        0,
                        0,
                        0,
                        &format!(
                            "<div class='text-[11px] text-rose-600 bg-rose-50 rounded-lg px-3 py-2 break-all'>{}</div>",
                            html_escape(&e)
                        ),
                    );
                }
            }
            Err(e) => {
                if idx == 0 {
                    return result_page(
                        "Fixture Sync",
                        "Sync task error",
                        0,
                        0,
                        0,
                        0,
                        &html_escape(&e.to_string()),
                    );
                }
            }
        }
    }

    let mut imported = 0i32;
    let mut skipped = 0i32;

    for ev in events {
        if ev.home.is_empty() || ev.away.is_empty() {
            continue;
        }
        if !league_allowed(&ev.league, &ev.league_id, &allowed, &allowed_ids) {
            continue;
        }
        let date = if ev.date.is_empty() {
            requested_date.clone()
        } else {
            ev.date.clone()
        };
        let date = date.chars().take(10).collect::<String>();
        if date.len() < 10 {
            continue;
        }
        let time = ev
            .time
            .chars()
            .take(8)
            .filter(|c| c.is_ascii_digit() || *c == ':')
            .collect::<String>();
        let time = if time.is_empty() {
            timeutil::MATCH_DEFAULT_TIME.to_string()
        } else {
            time
        };
        let match_time = format!("{} {}", date, time);

        // Keep only the events that belong to the requested matchday.
        let belongs = timeutil::parse_dt(&match_time)
            .map(|dt| timeutil::business_date_of(dt) == requested_date)
            .unwrap_or(false);
        if !belongs {
            continue;
        }

        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM matches WHERE date(match_time)=?1 AND home_team=?2 AND away_team=?3",
        )
        .bind(&date)
        .bind(&ev.home)
        .bind(&ev.away)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

        if exists > 0 {
            skipped += 1;
            continue;
        }

        let res = sqlx::query(
            "INSERT INTO matches (match_time, home_team, away_team, upper_team, body_odd, total_odd, status) \
             VALUES (?1, ?2, ?3, 'HOME', '0-0', '0-0', 'OPEN')",
        )
        .bind(&match_time)
        .bind(&ev.home)
        .bind(&ev.away)
        .execute(&state.db)
        .await;

        if res.is_ok() {
            imported += 1;
        }
    }

    result_page(
        "Fixture Sync",
        "Import ပြီးပါပြီ။ Odds များကို Paste ဖြင့် ဖြည့်နိုင်ပါသည်။",
        imported,
        0,
        skipped,
        0,
        "",
    )
}

// ============================ PASTE & PARSE ============================

struct ParsedInsert {
    date: String,
    home: String,
    away: String,
    upper_team: String,
    body_odd: String,
    total_odd: String,
}

struct ParsedOdds {
    date: String,
    fav: String,
    body_odd: String,
    total_odd: String,
}

#[derive(sqlx::FromRow)]
struct MatchTeams {
    id: i64,
    home_team: String,
    away_team: String,
}

fn is_date_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

/// Split an optional leading `YYYY-MM-DD` off a line.
pub fn split_leading_date(line: &str) -> (String, Option<String>) {
    let text = line.trim();
    if is_date_prefix(text) {
        (text[10..].trim_start().to_string(), Some(text[..10].to_string()))
    } else {
        (text.to_string(), None)
    }
}

fn parse_odds_token(token: &str) -> Result<(String, String), String> {
    let (body, total) = token
        .split_once('/')
        .ok_or("ပေါက်ဈေး format မှားနေပါသည်")?;
    let body = body.trim().to_string();
    let total = total.trim().to_string();
    if body.is_empty() || total.is_empty() {
        return Err("ပေါက်ဈေး format မှားနေပါသည်".to_string());
    }
    Ok((body, total))
}

/// `FavTeam bodyOdd/totalOdd`
fn parse_odds_line(line: &str, default_date: &str) -> Result<ParsedOdds, String> {
    let (text, date_opt) = split_leading_date(line);
    let date = date_opt.unwrap_or_else(|| default_date.to_string());
    if text.is_empty() {
        return Err("empty".to_string());
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let odds_pos = tokens
        .iter()
        .rposition(|t| t.contains('/'))
        .ok_or("ပေါက်ဈေး (bodyOdd/totalOdd) မတွေ့ပါ")?;
    let (body, total) = parse_odds_token(tokens[odds_pos])?;
    let fav = tokens[..odds_pos].join(" ");
    if fav.trim().is_empty() {
        return Err("Fav အသင်းနာမည် မပါပါ".to_string());
    }
    Ok(ParsedOdds { date, fav: fav.trim().to_string(), body_odd: body, total_odd: total })
}

/// `Home vs Away [FavTeam] bodyOdd/totalOdd`
fn parse_insert_line(line: &str, default_date: &str) -> Result<ParsedInsert, String> {
    let (text, date_opt) = split_leading_date(line);
    let date = date_opt.unwrap_or_else(|| default_date.to_string());
    if text.is_empty() {
        return Err("empty".to_string());
    }
    let lower = text.to_lowercase();
    let idx = lower.find(" vs ").ok_or("' vs ' မပါပါ")?;
    let home = text[..idx].trim().to_string();
    let rest = text[idx + 4..].trim().to_string();
    if home.is_empty() {
        return Err("Home team မပါပါ".to_string());
    }
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let odds_pos = tokens
        .iter()
        .rposition(|t| t.contains('/'))
        .ok_or("ပေါက်ဈေး (bodyOdd/totalOdd) မတွေ့ပါ")?;
    let (body, total) = parse_odds_token(tokens[odds_pos])?;
    let teams_fav = tokens[..odds_pos].join(" ");
    if teams_fav.is_empty() {
        return Err("Away team မပါပါ".to_string());
    }

    let home_lower = home.to_lowercase();
    let teams_fav_lower = teams_fav.to_lowercase();
    let (away, fav) = if teams_fav_lower.ends_with(&home_lower) && teams_fav.len() > home.len() {
        let away = teams_fav[..teams_fav.len() - home.len()].trim().to_string();
        if away.is_empty() {
            return Err("Away team မပါပါ".to_string());
        }
        (away, home.clone())
    } else {
        let parts: Vec<&str> = teams_fav.split_whitespace().collect();
        if parts.len() >= 2 && parts.len() % 2 == 0 {
            let half = parts.len() / 2;
            let a = parts[..half].join(" ");
            let b = parts[half..].join(" ");
            if a.eq_ignore_ascii_case(&b) {
                (a.clone(), a)
            } else {
                (teams_fav.clone(), home.clone())
            }
        } else {
            (teams_fav.clone(), home.clone())
        }
    };
    let upper_team = if fav.eq_ignore_ascii_case(&home) { "HOME" } else { "AWAY" };

    Ok(ParsedInsert {
        date,
        home,
        away,
        upper_team: upper_team.to_string(),
        body_odd: body,
        total_odd: total,
    })
}

// ---- fuzzy team matching ----

pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let dist = levenshtein(&ac, &bc) as f64;
    let max_len = ac.len().max(bc.len()) as f64;
    1.0 - dist / max_len
}

/// Find the fixture (on `date`) whose home or away team best matches `fav`.
/// Returns (match_id, is_home).
pub async fn find_match_for_fav(state: &AppState, date: &str, fav: &str) -> Option<(i64, bool)> {
    let rows = sqlx::query_as::<_, MatchTeams>(
        "SELECT id, home_team, away_team FROM matches \
         WHERE date(match_time)=?1 AND status != 'FINISHED'",
    )
    .bind(date)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let fav_n = normalize(fav);
    let mut best: Option<(f64, i64, bool)> = None;
    for row in rows {
        let h = similarity(&fav_n, &normalize(&row.home_team));
        let a = similarity(&fav_n, &normalize(&row.away_team));
        let (score, is_home) = if h >= a { (h, true) } else { (a, false) };
        if best.map(|(s, _, _)| score > s).unwrap_or(true) {
            best = Some((score, row.id, is_home));
        }
    }
    match best {
        Some((score, id, is_home)) if score >= FAV_MATCH_THRESHOLD => Some((id, is_home)),
        _ => None,
    }
}

pub async fn parse_matches(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }

    let pasted = form.get("pasted").cloned().unwrap_or_default();
    let default_date = form
        .get("match_date")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(timeutil::business_today);

    let mut added = 0i32;
    let mut updated = 0i32;
    let mut skipped = 0i32;
    let mut failed = 0i32;
    let mut errors = String::new();

    for raw_line in pasted.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (detect, _) = split_leading_date(line);
        let is_insert = detect.to_lowercase().contains(" vs ");

        if is_insert {
            match parse_insert_line(line, &default_date) {
                Ok(pm) => {
                    let match_time = format!("{} {}", pm.date, timeutil::MATCH_DEFAULT_TIME);
                    let exists: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM matches WHERE date(match_time)=?1 AND home_team=?2 AND away_team=?3",
                    )
                    .bind(&pm.date)
                    .bind(&pm.home)
                    .bind(&pm.away)
                    .fetch_one(&state.db)
                    .await
                    .unwrap_or(0);
                    if exists > 0 {
                        skipped += 1;
                        continue;
                    }
                    let res = sqlx::query(
                        "INSERT INTO matches (match_time, home_team, away_team, upper_team, body_odd, total_odd, status) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'OPEN')",
                    )
                    .bind(&match_time)
                    .bind(&pm.home)
                    .bind(&pm.away)
                    .bind(&pm.upper_team)
                    .bind(&pm.body_odd)
                    .bind(&pm.total_odd)
                    .execute(&state.db)
                    .await;
                    if res.is_ok() {
                        added += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(e) => {
                    failed += 1;
                    errors.push_str(&err_row(line, &e));
                }
            }
        } else {
            match parse_odds_line(line, &default_date) {
                Ok(po) => match find_match_for_fav(&state, &po.date, &po.fav).await {
                    Some((match_id, is_home)) => {
                        let upper = if is_home { "HOME" } else { "AWAY" };
                        let res = sqlx::query(
                            "UPDATE matches SET upper_team=?1, body_odd=?2, total_odd=?3 WHERE id=?4",
                        )
                        .bind(upper)
                        .bind(&po.body_odd)
                        .bind(&po.total_odd)
                        .bind(match_id)
                        .execute(&state.db)
                        .await;
                        if res.is_ok() {
                            updated += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    None => {
                        failed += 1;
                        errors.push_str(&err_row(
                            line,
                            &format!("Fav '{}' နှင့် ကိုက်ညီသောပွဲ မတွေ့ပါ ({})", po.fav, po.date),
                        ));
                    }
                },
                Err(e) => {
                    failed += 1;
                    errors.push_str(&err_row(line, &e));
                }
            }
        }
    }

    result_page(
        "Paste Odds",
        "Fav အသင်းနာမည်ဖြင့် ရှာပြီး ပေါက်ဈေး ထည့်ပြီး/ပွဲအသစ် ထည့်ပြီးပါပြီ။",
        added,
        updated,
        skipped,
        failed,
        &errors,
    )
}

fn err_row(line: &str, msg: &str) -> String {
    format!(
        "<div class='text-[11px] text-rose-600 bg-rose-50 rounded-lg px-3 py-1.5 break-all'>{}</div>",
        html_escape(&format!("{line} — {msg}"))
    )
}

// ============================ AUTO RESULTS ============================

fn fixtures_api_url() -> String {
    std::env::var("FIXTURES_API_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FIXTURES_URL.to_string())
}

/// Fetch final scores for OPEN/LIVE matches on `date` and settle them.
/// Returns (settled, checked).
pub async fn sync_results_for_date(state: &AppState, date: &str) -> (i32, i32) {
    let url = fixtures_api_url().replace("{date}", date);
    let events = match tokio::task::spawn_blocking(move || fetch_fixtures(&url)).await {
        Ok(Ok(events)) => events,
        _ => return (0, 0),
    };

    let matches = sqlx::query_as::<_, Match>(
        "SELECT * FROM matches WHERE date(match_time)=?1 AND status IN ('OPEN','LIVE')",
    )
    .bind(date)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let checked = matches.len() as i32;
    let mut settled = 0i32;

    for m in &matches {
        let hn = normalize(&m.home_team);
        let an = normalize(&m.away_team);
        let mut best: Option<(f64, i32, i32)> = None;
        for ev in &events {
            let (Some(hs), Some(ags)) = (ev.home_score, ev.away_score) else {
                continue;
            };
            let score =
                similarity(&hn, &normalize(&ev.home)).min(similarity(&an, &normalize(&ev.away)));
            if best.map(|(b, _, _)| score > b).unwrap_or(true) {
                best = Some((score, hs as i32, ags as i32));
            }
        }
        if let Some((score, hs, ags)) = best {
            if score >= FAV_MATCH_THRESHOLD
                && crate::handlers::admin::settle_match(&state.db, m, hs, ags)
                    .await
                    .is_ok()
            {
                settled += 1;
            }
        }
    }

    (settled, checked)
}

async fn run_result_sync(state: &AppState) {
    let today = timeutil::business_today();
    let dates: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT date(match_time) FROM matches WHERE status IN ('OPEN','LIVE')",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for d in dates {
        if d.as_str() <= today.as_str() {
            let _ = sync_results_for_date(state, &d).await;
        }
    }
}

/// Background loop that periodically settles finished matches from the API.
/// Disable with RESULT_SYNC_INTERVAL_MIN=0.
pub fn spawn_result_sync(state: AppState) {
    let interval_min: u64 = std::env::var("RESULT_SYNC_INTERVAL_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    if interval_min == 0 {
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(interval_min * 60)).await;
            run_result_sync(&state).await;
        }
    });
}

fn results_page(settled: i32, checked: i32, date: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Sync Results - Chan Sport</title>
<script src="https://cdn.tailwindcss.com"></script></head>
<body class="bg-[#eef2f6] text-slate-800 font-sans min-h-screen flex items-center justify-center p-4">
<div class="bg-white rounded-2xl border border-slate-200 shadow-sm max-w-md w-full p-6 space-y-3">
<div class="text-lg font-black text-[#0b2545] text-center">Sync Results</div>
<div class="text-sm text-slate-600 text-center">{} ရက်အတွက် ပွဲရလဒ်များ စစ်ဆေးပြီးပါပြီ။</div>
<div class="grid grid-cols-2 gap-2 text-center text-xs">
<div class="px-3 py-1.5 rounded-lg bg-emerald-50 text-emerald-700 font-bold">Settled<br>{settled}</div>
<div class="px-3 py-1.5 rounded-lg bg-slate-100 text-slate-600 font-bold">Running Checked<br>{checked}</div>
</div>
<a href="/admin/result" class="block py-3 bg-[#0b2545] text-amber-300 font-bold rounded-lg text-sm text-center">Back to Result</a>
</div></body></html>"#,
        html_escape(date)
    );
    Html(html).into_response()
}

pub async fn sync_results(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if require_admin(&state, &jar).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    let date = form
        .get("sync_date")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(timeutil::business_today);
    let (settled, checked) = sync_results_for_date(&state, &date).await;
    results_page(settled, checked, &date)
}

#[cfg(test)]
mod tests {
    use super::{normalize, parse_insert_line, parse_odds_line, similarity, FAV_MATCH_THRESHOLD};

    #[test]
    fn odds_line_fav_only() {
        let p = parse_odds_line("Arsenal -20/3+50", "2026-10-11").unwrap();
        assert_eq!(p.fav, "Arsenal");
        assert_eq!(p.body_odd, "-20");
        assert_eq!(p.total_odd, "3+50");
        assert_eq!(p.date, "2026-10-11");
    }

    #[test]
    fn odds_line_multiworld_fav_with_date() {
        let p = parse_odds_line("2026-12-05 Man City 1+25/3-50", "2026-01-01").unwrap();
        assert_eq!(p.fav, "Man City");
        assert_eq!(p.date, "2026-12-05");
    }

    #[test]
    fn insert_line_home_favorite() {
        let p = parse_insert_line("Arsenal vs Man City Arsenal -20/3+50", "2026-10-11").unwrap();
        assert_eq!(p.home, "Arsenal");
        assert_eq!(p.away, "Man City");
        assert_eq!(p.upper_team, "HOME");
    }

    #[test]
    fn insert_line_away_favorite() {
        let p = parse_insert_line("Arsenal vs Man City Man City 1+25/3-50", "2026-10-11").unwrap();
        assert_eq!(p.upper_team, "AWAY");
    }

    #[test]
    fn fuzzy_similarity() {
        assert!(similarity(&normalize("ManCity"), &normalize("Man City")) > 0.99);
        assert!(similarity(&normalize("Arsnal"), &normalize("Arsenal")) > FAV_MATCH_THRESHOLD);
        assert!(similarity(&normalize("Arsenal"), &normalize("Chelsea")) < FAV_MATCH_THRESHOLD);
    }
}
