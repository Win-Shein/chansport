//! MYANMAR FOOTBALL ODDS SETTLEMENT ENGINE (Rust port)
//!
//! This module is a faithful port of the original Node.js `settlement.js`.
//! It parses Myanmar-style odds such as `1-50`, `0.5+25`, `2.5-80`, `0-0`
//! and settles a bet against a final score.

/// Any winning profit (full win OR a partial "ပြားစွန်း" handicap-fraction win)
/// is paid at 95%, not 100% — the house keeps a 5% cut on winnings only, never on losses.
pub const WIN_PROFIT_RATE: f64 = 0.95;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedOdd {
    pub hdc: f64,
    pub sign: char,
    pub val: i64,
}

/// Parse a Myanmar odd string of the shape `[hdc][+|-][val]`.
///
/// Mirrors the JS regex `^([0-9.]+)?([+-])([0-9]+)$`, then falls back to a
/// plain-number parse (handicap only, sign '-', val 0) when there is no sign.
pub fn parse_myanmar_odd(odd_str: &str) -> ParsedOdd {
    let s = odd_str.trim();
    let bytes = s.as_bytes();

    // Locate the sign. It may only appear at index 0 when there is no handicap part.
    let mut sign_idx: Option<usize> = None;
    if !bytes.is_empty() && (bytes[0] == b'+' || bytes[0] == b'-') {
        sign_idx = Some(0);
    } else {
        for (i, &b) in bytes.iter().enumerate().skip(1) {
            if b == b'+' || b == b'-' {
                sign_idx = Some(i);
                break;
            }
        }
    }

    if let Some(i) = sign_idx {
        let hdc_str = &s[..i];
        let sign = s[i..].chars().next().unwrap_or('-');
        let val_str = &s[i + 1..];

        let val_ok = !val_str.is_empty() && val_str.bytes().all(|b| b.is_ascii_digit());
        let hdc_ok = hdc_str.is_empty()
            || hdc_str
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.');

        if val_ok && hdc_ok {
            let hdc = if hdc_str.is_empty() {
                0.0
            } else {
                hdc_str.parse::<f64>().unwrap_or(0.0)
            };
            let val = val_str.parse::<i64>().unwrap_or(0);
            return ParsedOdd { hdc, sign, val };
        }
    }

    // Fallback: plain handicap number (e.g. "2.5")
    let hdc = s.parse::<f64>().unwrap_or(0.0);
    ParsedOdd {
        hdc: if hdc.is_nan() { 0.0 } else { hdc },
        sign: '-',
        val: 0,
    }
}

/// Format a handicap as JS would: `0.5` -> "0.5", `1` -> "1", `2.50` -> "2.5".
fn format_hdc(hdc: f64) -> String {
    if hdc.fract() == 0.0 {
        format!("{}", hdc as i64)
    } else {
        let mut s = format!("{:.2}", hdc);
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

/// Return the opposite side of a Myanmar odd (flip the +/- sign).
pub fn get_opposite_odd(odd_str: &str) -> String {
    let parsed = parse_myanmar_odd(odd_str);
    let opp_sign = if parsed.sign == '-' { '+' } else { '-' };
    let hdc_part = if parsed.hdc > 0.0 {
        format_hdc(parsed.hdc)
    } else {
        String::new()
    };
    format!("{}{}{}", hdc_part, opp_sign, parsed.val)
}

/// Bet status values (kept as &'static str to match the original strings).
pub const STATUS_WON: &str = "WON";
pub const STATUS_LOST: &str = "LOST";
pub const STATUS_DRAW: &str = "DRAW";

#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    pub status: &'static str,
    pub payout: f64,
    pub valid_amount: f64,
}

#[derive(Debug, Clone)]
pub struct BetInput {
    pub amount: f64,
    pub choice: String,
    pub bet_type: String,
    pub odd: String,
    pub home_team: String,
    pub away_team: String,
    pub upper_team: String,
}

/// Settle a single bet against the final score.
pub fn settle_myanmar_bet(bet: &BetInput, home_score: i32, away_score: i32) -> Settlement {
    let amount = bet.amount;
    let choice = bet.choice.trim().to_uppercase();
    let bet_type = bet.bet_type.as_str();
    let parsed_odd = parse_myanmar_odd(&bet.odd);
    let rate = parsed_odd.val as f64 / 100.0;

    let mut is_full_win = false;
    let mut is_full_lost = false;
    let mut is_level = false;

    // 1. OVER / UNDER
    if bet_type == "OVER_UNDER" {
        let total_goals = (home_score + away_score) as f64;
        let hdc = parsed_odd.hdc;
        let is_bet_over = choice.contains("OVER");

        if total_goals > hdc {
            if is_bet_over {
                is_full_win = true;
            } else {
                is_full_lost = true;
            }
        } else if total_goals < hdc {
            if is_bet_over {
                is_full_lost = true;
            } else {
                is_full_win = true;
            }
        } else {
            is_level = true;
        }
    }

    // 2. BODY
    if bet_type == "BODY" {
        let hdc = parsed_odd.hdc;
        let is_home_upper = bet.upper_team == "HOME";
        let home_upper = bet.home_team.to_uppercase();
        let is_bet_home = choice.contains(&home_upper) || choice.contains("HOME");
        let is_bet_upper = if is_home_upper { is_bet_home } else { !is_bet_home };

        let goal_diff = if is_home_upper {
            (home_score - away_score) as f64
        } else {
            (away_score - home_score) as f64
        };

        if goal_diff > hdc {
            if is_bet_upper {
                is_full_win = true;
            } else {
                is_full_lost = true;
            }
        } else if goal_diff < hdc {
            if is_bet_upper {
                is_full_lost = true;
            } else {
                is_full_win = true;
            }
        } else {
            is_level = true;
        }
    }

    // 3. PAYOUT & VALID AMOUNT CALCULATION

    // အပြည့်နိုင် (မူရင်းလောင်းကြေး + အမြတ် ၉၅%)
    if is_full_win {
        return Settlement {
            status: STATUS_WON,
            payout: amount + (amount * WIN_PROFIT_RATE),
            valid_amount: amount,
        };
    }

    // အပြည့်ရှုံး
    if is_full_lost {
        return Settlement {
            status: STATUS_LOST,
            payout: 0.0,
            valid_amount: amount,
        };
    }

    // ကြေးကျချိန် (ပြားစွန်းအလိုက် စား/ပေး တွက်ချက်ခြင်း)
    if is_level {
        if parsed_odd.val == 0 {
            return Settlement {
                status: STATUS_DRAW,
                payout: amount,
                valid_amount: 0.0,
            };
        }

        // ပြားစွန်းအလိုက် အမှန်တကယ် လောင်းကြေးဖြစ်သော ပမာဏ
        let effective_stake = amount * rate;

        if parsed_odd.sign == '-' {
            // အနုတ်ကြေး: ရှုံးသည် (ကျန်ငွေ ပြန်အမ်း)
            return Settlement {
                status: STATUS_LOST,
                payout: amount - effective_stake,
                valid_amount: effective_stake,
            };
        } else {
            // အပေါင်းကြေး: နိုင်သည် (မူရင်းလောင်းကြေး + ပြားစွန်းအမြတ် ၉၅%)
            return Settlement {
                status: STATUS_WON,
                payout: amount + (effective_stake * WIN_PROFIT_RATE),
                valid_amount: effective_stake,
            };
        }
    }

    Settlement {
        status: STATUS_DRAW,
        payout: amount,
        valid_amount: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bet(amount: f64, choice: &str, bet_type: &str, odd: &str, home: &str, away: &str, upper: &str) -> BetInput {
        BetInput {
            amount,
            choice: choice.to_string(),
            bet_type: bet_type.to_string(),
            odd: odd.to_string(),
            home_team: home.to_string(),
            away_team: away.to_string(),
            upper_team: upper.to_string(),
        }
    }

    #[test]
    fn parses_handicap_and_value() {
        assert_eq!(parse_myanmar_odd("1-50"), ParsedOdd { hdc: 1.0, sign: '-', val: 50 });
        assert_eq!(parse_myanmar_odd("0.5+25"), ParsedOdd { hdc: 0.5, sign: '+', val: 25 });
        assert_eq!(parse_myanmar_odd("2.5-80"), ParsedOdd { hdc: 2.5, sign: '-', val: 80 });
        assert_eq!(parse_myanmar_odd("-50").val, 50);
        assert_eq!(parse_myanmar_odd("-50").hdc, 0.0);
        assert_eq!(parse_myanmar_odd("2.5").hdc, 2.5);
        assert_eq!(parse_myanmar_odd("0-0").val, 0);
    }

    #[test]
    fn opposite_odd_flips_sign() {
        assert_eq!(get_opposite_odd("1-50"), "1+50");
        assert_eq!(get_opposite_odd("0.5+25"), "0.5-25");
        assert_eq!(get_opposite_odd("2.5-80"), "2.5+80");
    }

    #[test]
    fn over_full_win_pays_full_stake_plus_95pct() {
        let b = bet(10000.0, "OVER (Over)", "OVER_UNDER", "2.5-80", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 2, 1); // total 3 > 2.5 -> over wins
        assert_eq!(r.status, STATUS_WON);
        assert_eq!(r.payout, 10000.0 + 10000.0 * 0.95);
        assert_eq!(r.valid_amount, 10000.0);
    }

    #[test]
    fn over_full_lost_keeps_nothing() {
        let b = bet(10000.0, "OVER (Over)", "OVER_UNDER", "2.5-80", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 1, 1); // total 2 < 2.5 -> over loses
        assert_eq!(r.status, STATUS_LOST);
        assert_eq!(r.payout, 0.0);
        assert_eq!(r.valid_amount, 10000.0);
    }

    #[test]
    fn under_below_line_full_win() {
        let b = bet(10000.0, "UNDER (Under)", "OVER_UNDER", "2.5-80", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 1, 1); // total 2 < 2.5 -> under wins
        assert_eq!(r.status, STATUS_WON);
        assert_eq!(r.payout, 10000.0 + 10000.0 * 0.95);
        assert_eq!(r.valid_amount, 10000.0);
    }

    #[test]
    fn under_level_exact_handicap() {
        let b = bet(10000.0, "UNDER (Under)", "OVER_UNDER", "3-80", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 2, 1); // total 3 == hdc 3 -> level
        assert_eq!(r.status, STATUS_LOST);
        assert_eq!(r.payout, 10000.0 - 10000.0 * 0.8);
        assert_eq!(r.valid_amount, 8000.0);
    }

    #[test]
    fn under_level_plus_odd_wins_partial() {
        let b = bet(10000.0, "UNDER (Under)", "OVER_UNDER", "3+80", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 2, 1); // total 3 == hdc 3 -> level, plus -> win partial
        assert_eq!(r.status, STATUS_WON);
        assert_eq!(r.payout, 10000.0 + 8000.0 * 0.95);
        assert_eq!(r.valid_amount, 8000.0);
    }

    #[test]
    fn level_with_zero_value_is_draw() {
        let b = bet(10000.0, "UNDER (Under)", "OVER_UNDER", "3-0", "A", "B", "HOME");
        let r = settle_myanmar_bet(&b, 3, 0);
        assert_eq!(r.status, STATUS_DRAW);
        assert_eq!(r.payout, 10000.0);
        assert_eq!(r.valid_amount, 0.0);
    }

    #[test]
    fn body_home_upper_home_wins() {
        let b = bet(50000.0, "Arsenal", "BODY", "1-50", "Arsenal", "Chelsea", "HOME");
        let r = settle_myanmar_bet(&b, 3, 1); // diff 2 > 1
        assert_eq!(r.status, STATUS_WON);
        assert_eq!(r.payout, 50000.0 + 50000.0 * 0.95);
    }

    #[test]
    fn body_away_upper_underdog_covers() {
        // Away is upper. Betting home (underdog) with away giving 1 goal.
        let b = bet(50000.0, "Chelsea", "BODY", "1-50", "Arsenal", "Chelsea", "AWAY");
        // away upper: goal_diff = away - home = 2 - 1 = 1, hdc 1 -> level
        let r = settle_myanmar_bet(&b, 1, 2);
        assert_eq!(r.status, STATUS_LOST); // minus odd at level -> lose 50%
        assert_eq!(r.payout, 50000.0 - 50000.0 * 0.5);
    }
}
