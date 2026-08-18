const express = require('express');
const path = require('path');
const session = require('express-session');
const pgSession = require('connect-pg-simple')(session);
const helmet = require('helmet');
const rateLimit = require('express-rate-limit');
const bcrypt = require('bcryptjs');
const { pool, query } = require('./db');
const { settleMyanmarBet, getOppositeOdd } = require('./settlement');
require('dotenv').config();

const app = express();

// Hide framework fingerprint header (security)
app.disable('x-powered-by');

// Trust the first proxy hop (aaPanel/Nginx reverse proxy in front of this app).
// Required so req.ip / X-Forwarded-For and secure-cookie detection work correctly
// behind a reverse proxy, and so express-rate-limit doesn't reject requests.
app.set('trust proxy', 1);

// ===== SECURITY MIDDLEWARE =====
// Helmet sets secure HTTP response headers.
// CSP is disabled because the templates use Tailwind/Lucide CDNs
// + inline scripts (enabling it would otherwise break the UI).
app.use(helmet({ contentSecurityPolicy: false }));

// View Engine Setup (EJS)
app.set('view engine', 'ejs');
app.set('views', path.join(__dirname, 'views'));

// Body Parsers (with payload size limits to prevent large-payload attacks)
app.use(express.json({ limit: '100kb' }));
app.use(express.urlencoded({ extended: true, limit: '100kb' }));

// ===== RATE LIMITERS (anti brute-force / DoS) =====
// Global API limiter
const apiLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 200,
  standardHeaders: true,
  legacyHeaders: false,
  message: { success: false, message: 'Too many requests. Please try again later.' }
});

// Stricter limiter exclusively for login (anti brute-force)
const loginLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 10,
  standardHeaders: true,
  legacyHeaders: false,
  message: { success: false, message: 'Too many login attempts. Please try again after 15 minutes.' }
});

// Production detection (sets secure cookies when NODE_ENV=production)
const isProd = process.env.NODE_ENV === 'production';

// Session Setup (hardened cookie, persisted in Postgres so restarts/deploys
// don't wipe everyone's login and multiple app instances share sessions)
app.use(session({
  store: new pgSession({
    pool,
    tableName: 'session',
    createTableIfMissing: true
  }),
  name: 'myanbet.sid',
  secret: process.env.SESSION_SECRET || 'myanbet365_please_change_this_secret',
  resave: false,
  saveUninitialized: false,
  cookie: {
    maxAge: 24 * 60 * 60 * 1000, // 1 day
    httpOnly: true,              // block JS/cookie access (XSS)
    secure: isProd,              // only send over HTTPS in production
    sameSite: 'lax'              // CSRF mitigation for cross-site requests
  }
}));

// ===== INPUT SANITIZER & VALIDATORS =====
// Strips HTML angle brackets (prevents stored XSS) and limits length
function cleanString(val, maxLen = 200) {
  if (val === undefined || val === null) return '';
  return String(val).replace(/[<>]/g, '').trim().slice(0, maxLen);
}

// Validate a positive integer id
function isValidId(val) {
  return Number.isInteger(Number(val)) && Number(val) > 0;
}

// ===== SAFE ERROR HANDLER =====
// Logs the full error server-side, but never leaks DB/SQL internals to the client.
function fail(req, res, err, status = 500) {
  console.error(`[ERROR] ${req.method} ${req.originalUrl}:`, err.message);
  if (req.originalUrl.startsWith('/member/bet')) {
    return res.status(status).json({ success: false, message: 'System error. Please contact support.' });
  }
  return res.status(status).send('Something went wrong. Please try again.');
}

// ===== APP SETTINGS (Global Bet Lock) =====
async function ensureSettingsTable() {
  await query(`
    CREATE TABLE IF NOT EXISTS app_settings (
      id SERIAL PRIMARY KEY,
      betting_locked BOOLEAN NOT NULL DEFAULT FALSE
    );
  `);
  const existing = await query('SELECT id FROM app_settings LIMIT 1');
  if (existing.rows.length === 0) {
    await query('INSERT INTO app_settings (betting_locked) VALUES (FALSE)');
  }
}

async function isBettingLocked() {
  const res = await query('SELECT betting_locked FROM app_settings ORDER BY id LIMIT 1');
  return res.rows.length > 0 ? res.rows[0].betting_locked : false;
}

// Returns today's business date (YYYY-MM-DD) using the noon-to-noon cutoff
async function getBusinessToday() {
  const res = await query("SELECT DATE(NOW() + INTERVAL '12 hours')::text as today");
  return res.rows[0].today;
}

// Resolves the effective date filter for a "daily records" page:
// - ?filter_date=all      -> show everything, no date filter applied
// - ?filter_date=YYYY-MM-DD -> filter to that specific business day
// - no filter_date at all -> default to TODAY's business day (not "show all")
async function resolveFilterDate(rawFilterDate) {
  const showAll = rawFilterDate === 'all';
  const filter_date = showAll ? '' : (rawFilterDate || await getBusinessToday());
  return { filter_date, showAll };
}

// ===== CREDIT LOGS (Member Deposit / Withdraw history) =====
async function ensureCreditLogsTable() {
  await query(`
    CREATE TABLE IF NOT EXISTS credit_logs (
      id SERIAL PRIMARY KEY,
      user_id INT REFERENCES users(id) ON DELETE CASCADE,
      type VARCHAR(10) NOT NULL,
      amount NUMERIC(15, 2) NOT NULL,
      balance_after NUMERIC(15, 2) NOT NULL,
      note VARCHAR(200),
      admin_username VARCHAR(50),
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
    );
  `);
}

// --- Authentication Middlewares ---

const requireAuth = (req, res, next) => {
  if (!req.session.user) {
    return res.redirect('/login');
  }
  next();
};

const requireAdmin = (req, res, next) => {
  if (!req.session.user || req.session.user.role !== 'ADMIN') {
    return res.redirect('/login');
  }
  next();
};

// --- General Routes ---

app.get('/', (req, res) => {
  res.redirect('/login');
});

// Login Page (GET)
app.get('/login', (req, res) => {
  if (req.session.user) {
    return req.session.user.role === 'ADMIN' ? res.redirect('/admin/members') : res.redirect('/member');
  }
  res.render('login', { error: null });
});

// Login Submit (POST)
app.post('/login', loginLimiter, async (req, res) => {
  const username = cleanString(req.body.username, 60);
  const password = String(req.body.password || '');

  if (!username || !password) {
    return res.render('login', { error: 'Please enter username and password.' });
  }

  try {
    const result = await query(
      'SELECT * FROM users WHERE username = $1',
      [username]
    );

    if (result.rows.length === 0) {
      return res.render('login', { error: 'Invalid username or password' });
    }

    const user = result.rows[0];

    // Verify password against stored bcrypt hash
    const valid = await bcrypt.compare(password, user.password).catch(() => false);
    if (!valid) {
      return res.render('login', { error: 'Invalid username or password' });
    }

    if (user.status === 'SUSPENDED') {
      return res.render('login', { error: 'Your account has been suspended. Please contact Admin.' });
    }

    req.session.user = {
      id: user.id,
      username: user.username,
      role: user.role
    };

    if (user.role === 'ADMIN') {
      res.redirect('/admin/members');
    } else {
      res.redirect('/member');
    }
  } catch (err) {
    // Do not leak error details on login
    return fail(req, res, err, 500);
  }
});

// Logout (GET)
app.get('/logout', (req, res) => {
  req.session.destroy(() => {
    res.redirect('/login');
  });
});

// Change Password (shared handler — scoped to the logged-in session user, admin or member)
async function handleChangePassword(req, res) {
  const currentPassword = String(req.body.current_password || '');
  const newPassword = String(req.body.new_password || '');
  const userId = req.session.user.id;

  if (!currentPassword || !newPassword || newPassword.length < 6) {
    return res.status(400).json({ success: false, message: 'Current password is required and new password must be at least 6 characters.' });
  }

  try {
    const userRes = await query('SELECT password FROM users WHERE id = $1', [userId]);
    const user = userRes.rows[0];
    if (!user) {
      return res.status(403).json({ success: false, message: 'Access denied.' });
    }

    const valid = await bcrypt.compare(currentPassword, user.password).catch(() => false);
    if (!valid) {
      return res.status(400).json({ success: false, message: 'Current password is incorrect.' });
    }

    const newHash = await bcrypt.hash(newPassword, 10);
    await query('UPDATE users SET password = $1 WHERE id = $2', [newHash, userId]);

    return res.json({ success: true, message: 'Password updated successfully.' });
  } catch (err) {
    console.error(`[ERROR] ${req.method} ${req.originalUrl}:`, err.message);
    return res.status(500).json({ success: false, message: 'System error. Please try again.' });
  }
}

app.post('/admin/change-password', requireAdmin, apiLimiter, handleChangePassword);
app.post('/member/change-password', requireAuth, apiLimiter, handleChangePassword);

// --- Member Routes ---

// Member Dashboard (GET) - Tabs: sports, history, winlose
app.get('/member', requireAuth, async (req, res) => {
  const currentTab = req.query.tab || 'sports';
  const userId = req.session.user.id;

  try {
    const userRes = await query('SELECT * FROM users WHERE id = $1', [userId]);
    const bettingLocked = await isBettingLocked();
    // Win/Lose tab defaults to today's business day; ?filter_date=all shows everything
    const { filter_date, showAll } = await resolveFilterDate(req.query.filter_date);

    // 1. Active open/live matches whose scheduled kickoff time hasn't arrived yet
    const matchesRes = await query(`
      SELECT * FROM matches
      WHERE status IN ('OPEN', 'LIVE') AND match_time > NOW()
      ORDER BY match_time ASC
    `);

    // 2. Running Bets History (PENDING only)
    const runningBetsRes = await query(`
      SELECT b.*, m.home_team, m.away_team, m.match_time, m.body_odd, m.total_odd
      FROM bets b
      JOIN matches m ON b.match_id = m.id
      WHERE b.user_id = $1 AND b.status = 'PENDING'
      ORDER BY b.created_at DESC
    `, [userId]);

    // 3. Settled Win/Lose History (WON, LOST, DRAW)
    let winLoseQuery = `
      SELECT b.*, m.home_team, m.away_team, m.home_score, m.away_score, m.match_time
      FROM bets b
      JOIN matches m ON b.match_id = m.id
      WHERE b.user_id = $1 AND b.status IN ('WON', 'LOST', 'DRAW')
    `;
    let winLoseParams = [userId];

    if (filter_date) {
      // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
      winLoseQuery += ` AND DATE(b.created_at + INTERVAL '12 hours') = $2`;
      winLoseParams.push(filter_date);
    }
    winLoseQuery += ` ORDER BY b.created_at DESC`;

    const winLoseRes = await query(winLoseQuery, winLoseParams);

    // 3b. Own Deposit / Withdraw History (same date filter as the Win/Lose bets above)
    let creditLogQuery = `SELECT * FROM credit_logs WHERE user_id = $1`;
    const creditLogParams = [userId];
    if (filter_date) {
      // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
      creditLogQuery += ` AND DATE(created_at + INTERVAL '12 hours') = $2`;
      creditLogParams.push(filter_date);
    }
    creditLogQuery += ` ORDER BY created_at DESC`;

    const creditLogRes = await query(creditLogQuery, creditLogParams);

    // Calculate Summary Metrics (net result includes commission earned on valid amount)
    const commRate = Number(userRes.rows[0].commission || 0);
    const totalStake = winLoseRes.rows.reduce((sum, r) => sum + Number(r.amount || 0), 0);
    const totalPayout = winLoseRes.rows.reduce((sum, r) => sum + Number(r.payout || 0), 0);
    const totalValidAmount = winLoseRes.rows.reduce((sum, r) => sum + Number(r.valid_amount || 0), 0);
    const commAmount = (totalValidAmount * commRate) / 100;
    const netWinLose = (totalPayout - totalStake) + commAmount;

    res.render('member_dashboard', {
      user: userRes.rows[0],
      currentTab,
      filter_date,
      showAll,
      matches: matchesRes.rows,
      runningBets: runningBetsRes.rows,
      winLoseBets: winLoseRes.rows,
      creditLog: creditLogRes.rows,
      totalStake,
      totalValidAmount,
      commRate,
      commAmount,
      netWinLose,
      bettingLocked
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// Member Place Bet (POST) - Returns JSON for smooth AJAX interaction
app.post('/member/bet', requireAuth, apiLimiter, async (req, res) => {
  const match_id = req.body.match_id;
  const bet_type = String(req.body.bet_type || '');
  const choice = String(req.body.choice || '');
  const userId = req.session.user.id;
  const betAmount = parseFloat(req.body.amount);

  if (!isValidId(match_id)) {
    return res.status(400).json({ success: false, message: 'လောင်းကြေးတင်မည့် ပွဲစဉ် မမှန်ကန်ပါ။' });
  }

  if (!['BODY', 'OVER_UNDER'].includes(bet_type)) {
    return res.status(400).json({ success: false, message: 'လောင်းကြေး အမျိုးအစား မမှန်ကန်ပါ။' });
  }

  if (!betAmount || isNaN(betAmount) || betAmount <= 0) {
    return res.status(400).json({ success: false, message: 'လောင်းကြေး ပမာဏ မှားယွင်းနေပါသည်။' });
  }

  try {
    if (await isBettingLocked()) {
      return res.status(403).json({ success: false, message: 'စနစ်တွင် လောင်းကြေး ယာယီ ပိတ်ထားပါသည်။ ခဏစောင့်ပြီး ပြန်ကြိုးစားပါ။' });
    }

    const userRes = await query(
      'SELECT balance, status, COALESCE(bet_limit, 100000) as bet_limit FROM users WHERE id = $1',
      [userId]
    );
    const user = userRes.rows[0];

    if (!user) {
      return res.status(403).json({ success: false, message: 'Access denied.' });
    }

    if (user.status === 'SUSPENDED') {
      return res.status(403).json({ success: false, message: 'သင့်အကောင့်ကို ပိတ်ထားပါသဖြင့် လောင်း၍မရနိုင်ပါ။' });
    }

    // Match must exist and still be open for betting — never trust a client-supplied match_id blindly.
    // "betting_closed" is computed by Postgres itself (using the server's Myanmar-time session
    // clock) so we never have to compare timestamps in JS and risk a timezone mismatch.
    const matchRes = await query(
      `SELECT *, (match_time <= NOW() + INTERVAL '5 minutes') as betting_closed
       FROM matches WHERE id = $1 AND status IN ('OPEN', 'LIVE')`,
      [match_id]
    );
    const match = matchRes.rows[0];

    if (!match) {
      return res.status(400).json({ success: false, message: 'ဒီပွဲစဉ်သည် လောင်းကြေး လက်ခံခြင်း မရှိတော့ပါ။' });
    }

    // Close betting 5 minutes before kickoff
    if (match.betting_closed) {
      return res.status(400).json({ success: false, message: 'ပွဲစဉ် မစခင် ၅ မိနစ် အတွင်း လောင်းကြေး လက်ခံခြင်း မရှိတော့ပါ။' });
    }

    // Derive the authoritative odd/handicap from the match record itself.
    // The odd is NEVER taken from the client — otherwise a forged odd could
    // guarantee a win regardless of the match result.
    const isUpperHome = match.upper_team === 'HOME';
    let odd;
    if (bet_type === 'BODY') {
      const homeOdd = isUpperHome ? match.body_odd : getOppositeOdd(match.body_odd);
      const awayOdd = isUpperHome ? getOppositeOdd(match.body_odd) : match.body_odd;
      if (choice === match.home_team) odd = homeOdd;
      else if (choice === match.away_team) odd = awayOdd;
    } else {
      const overOdd = match.total_odd;
      const underOdd = getOppositeOdd(match.total_odd);
      if (choice === 'OVER (Over)') odd = overOdd;
      else if (choice === 'UNDER (Under)') odd = underOdd;
    }

    if (!odd) {
      return res.status(400).json({ success: false, message: 'ရွေးချယ်မှု မမှန်ကန်ပါ။' });
    }

    // 1. Check Max Bet Limit
    if (betAmount > parseFloat(user.bet_limit)) {
      return res.status(400).json({ 
        success: false, 
        message: `အများဆုံး လောင်းနိုင်သော ပမာဏမှာ ${Number(user.bet_limit).toLocaleString()} MMK ဖြစ်ပါသည်!` 
      });
    }

    // 2. Check Balance
    if (parseFloat(user.balance) < betAmount) {
      return res.status(400).json({ 
        success: false, 
        message: 'လက်ကျန်ငွေ မလုံလောက်ပါ။ ကျေးဇူးပြု၍ ငွေဖြည့်သွင်းပါ။' 
      });
    }

    // Deduct balance & record pending bet
    await query('UPDATE users SET balance = balance - $1 WHERE id = $2', [betAmount, userId]);
    await query(`
      INSERT INTO bets (user_id, match_id, bet_type, choice, odd, amount, status) 
      VALUES ($1, $2, $3, $4, $5, $6, 'PENDING')
    `, [userId, match_id, bet_type, choice, odd, betAmount]);

    return res.json({ success: true, message: 'လောင်းကြေး အောင်မြင်စွာ တင်ပြီးပါပြီ။' });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// --- Admin Modular Page Routes ---

app.get('/admin', requireAdmin, (req, res) => {
  res.redirect('/admin/members');
});

// 1. Member Management (GET)
app.get('/admin/members', requireAdmin, async (req, res) => {
  try {
    const membersRes = await query(
      "SELECT id, username, balance, COALESCE(commission, 0) as commission, COALESCE(bet_limit, 100000) as bet_limit, COALESCE(status, 'ACTIVE') as status, created_at FROM users WHERE role = 'MEMBER' ORDER BY id DESC"
    );
    const totalPoolRes = await query("SELECT SUM(balance) as total FROM users WHERE role = 'MEMBER'");

    res.render('admin_members', {
      admin: req.session.user,
      members: membersRes.rows,
      totalPool: totalPoolRes.rows[0].total || 0,
      page: 'members'
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// 2. Add Match Page (GET)
app.get('/admin/add-match', requireAdmin, (req, res) => {
  res.render('admin_add_match', {
    admin: req.session.user,
    page: 'add_match'
  });
});

// 3. Match List Page (GET) - defaults to today's business day; ?filter_date=all shows everything
app.get('/admin/match-list', requireAdmin, async (req, res) => {
  try {
    const { filter_date, showAll } = await resolveFilterDate(req.query.filter_date);

    let sqlQuery = `
      SELECT * FROM matches
      ORDER BY
        CASE
          WHEN status = 'LIVE' THEN 1
          WHEN status = 'OPEN' THEN 2
          ELSE 3
        END ASC,
        match_time ASC
    `;
    let queryParams = [];

    if (filter_date) {
      // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
      sqlQuery = `
        SELECT * FROM matches
        WHERE DATE(match_time + INTERVAL '12 hours') = $1
        ORDER BY
          CASE
            WHEN status = 'LIVE' THEN 1
            WHEN status = 'OPEN' THEN 2
            ELSE 3
          END ASC,
          match_time ASC
      `;
      queryParams.push(filter_date);
    }

    const matchesRes = await query(sqlQuery, queryParams);

    res.render('admin_match_list', {
      admin: req.session.user,
      matches: matchesRes.rows,
      filter_date,
      showAll,
      page: 'match_list'
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// 4. Result Page (GET) - defaults to today's business day; ?filter_date=all shows everything
app.get('/admin/result', requireAdmin, async (req, res) => {
  try {
    const { filter_date, showAll } = await resolveFilterDate(req.query.filter_date);

    let sqlQuery = `
      SELECT * FROM matches
      ORDER BY
        CASE
          WHEN home_score IS NULL THEN 1
          ELSE 2
        END ASC,
        match_time DESC
    `;
    let queryParams = [];

    if (filter_date) {
      // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
      sqlQuery = `
        SELECT * FROM matches
        WHERE DATE(match_time + INTERVAL '12 hours') = $1
        ORDER BY
          CASE
            WHEN home_score IS NULL THEN 1
            ELSE 2
          END ASC,
          match_time DESC
      `;
      queryParams.push(filter_date);
    }

    const matchesRes = await query(sqlQuery, queryParams);

    res.render('admin_result', {
      admin: req.session.user,
      matches: matchesRes.rows,
      filter_date,
      showAll,
      page: 'result'
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// 5. Report Page (GET) - Credit, Outstanding, Win/Lose with Valid Amount & Commission
app.get('/admin/report', requireAdmin, async (req, res) => {
  const subTab = req.query.tab || 'credit';

  try {
    // Credit Log / Win-Lose tabs default to today's business day; ?filter_date=all shows everything.
    // The Outstanding tab ignores this entirely (it's live PENDING bets, not a historical daily record).
    const { filter_date, showAll } = await resolveFilterDate(req.query.filter_date);

    // 1. Credit Data
    const creditMembersRes = await query(
      "SELECT id, username, balance, COALESCE(commission, 0) as commission, COALESCE(bet_limit, 100000) as bet_limit, COALESCE(status, 'ACTIVE') as status, created_at FROM users WHERE role = 'MEMBER' ORDER BY balance DESC"
    );

    // 1b. Credit Log (Deposit / Withdraw history, exact timestamps, per-member breakdown on click)
    // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
    const creditLogsQuery = `
      SELECT cl.*, u.username
      FROM credit_logs cl
      JOIN users u ON cl.user_id = u.id
      ${filter_date ? "WHERE DATE(cl.created_at + INTERVAL '12 hours') = $1" : ''}
      ORDER BY cl.created_at DESC
    `;
    const creditLogsRes = await query(creditLogsQuery, filter_date ? [filter_date] : []);

    // 2. Outstanding Data
    const outstandingRes = await query(`
      SELECT b.*, u.username, m.home_team, m.away_team, m.match_time, m.body_odd, m.total_odd
      FROM bets b
      JOIN users u ON b.user_id = u.id
      JOIN matches m ON b.match_id = m.id
      WHERE m.status IN ('OPEN', 'LIVE') AND b.status = 'PENDING'
      ORDER BY b.created_at DESC
    `);

    // 3. Settled Bets (All Detailed Slips for Modal Breakdown)
    // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
    const detailedBetsQuery = `
      SELECT b.*, u.username, m.home_team, m.away_team, m.home_score, m.away_score, m.match_time
      FROM bets b
      JOIN users u ON b.user_id = u.id
      JOIN matches m ON b.match_id = m.id
      WHERE b.status IN ('WON', 'LOST', 'DRAW')
      ${filter_date ? "AND DATE(b.created_at + INTERVAL '12 hours') = $1" : ''}
      ORDER BY b.created_at DESC
    `;
    const detailedBetsRes = await query(detailedBetsQuery, filter_date ? [filter_date] : []);

    // 4. Win/Lose Aggregated Member Data by Valid Amount
    // Business day runs noon-to-noon (cutoff 11:59 AM), not midnight-to-midnight
    let winLoseQuery = `
      SELECT
        u.id as user_id,
        u.username,
        COALESCE(u.commission, 0) as comm_rate,
        COUNT(b.id) as total_bets,
        COALESCE(SUM(b.amount), 0) as turnover,
        COALESCE(SUM(b.payout), 0) as payout,
        COALESCE(SUM(b.valid_amount), 0) as valid_amount
      FROM users u
      LEFT JOIN bets b ON u.id = b.user_id AND b.status IN ('WON', 'LOST', 'DRAW')
      ${filter_date ? "AND DATE(b.created_at + INTERVAL '12 hours') = $1" : ''}
      WHERE u.role = 'MEMBER'
      GROUP BY u.id, u.username, u.commission
      ORDER BY turnover DESC
    `;
    const winLoseParams = filter_date ? [filter_date] : [];
    const winLoseRes = await query(winLoseQuery, winLoseParams);

    const winLoseList = winLoseRes.rows.map(row => {
      const turnover = parseFloat(row.turnover) || 0;
      const payout = parseFloat(row.payout) || 0;
      const validAmount = parseFloat(row.valid_amount) || 0;
      const commRate = parseFloat(row.comm_rate) || 0;

      // Commission calculation based on Valid Amount
      const commAmount = (validAmount * commRate) / 100;
      // Member Net Result = Payout - Turnover + Comm Amount
      const memberNet = (payout - turnover) + commAmount;

      return {
        ...row,
        valid_amount: validAmount,
        comm_amount: commAmount,
        member_net: memberNet
      };
    });

    // 5. Outstanding Bets grouped by Member (table shows one row per member;
    // individual slips are revealed in a modal when that member is clicked)
    const outstandingByMemberMap = new Map();
    for (const bet of outstandingRes.rows) {
      const key = bet.user_id;
      if (!outstandingByMemberMap.has(key)) {
        outstandingByMemberMap.set(key, {
          user_id: bet.user_id,
          username: bet.username,
          total_bets: 0,
          total_stake: 0
        });
      }
      const entry = outstandingByMemberMap.get(key);
      entry.total_bets += 1;
      entry.total_stake += Number(bet.amount || 0);
    }
    const outstandingByMember = Array.from(outstandingByMemberMap.values())
      .sort((a, b) => b.total_stake - a.total_stake);

    // 6. Credit Log totals per member (Deposited / Withdrawn), for the summary table;
    // the full timestamped log is revealed in a modal when that member is clicked
    const creditTotalsMap = new Map();
    for (const log of creditLogsRes.rows) {
      const key = log.user_id;
      if (!creditTotalsMap.has(key)) {
        creditTotalsMap.set(key, { total_deposit: 0, total_withdraw: 0 });
      }
      const entry = creditTotalsMap.get(key);
      if (log.type === 'DEPOSIT') entry.total_deposit += Number(log.amount || 0);
      else entry.total_withdraw += Number(log.amount || 0);
    }
    const creditMembers = creditMembersRes.rows.map(m => ({
      ...m,
      total_deposit: creditTotalsMap.get(m.id)?.total_deposit || 0,
      total_withdraw: creditTotalsMap.get(m.id)?.total_withdraw || 0
    }));

    res.render('admin_report', {
      admin: req.session.user,
      subTab,
      filter_date,
      showAll,
      creditMembers,
      creditLogs: creditLogsRes.rows,
      outstandingBets: outstandingRes.rows,
      outstandingByMember,
      winLoseList,
      allSettledBets: detailedBetsRes.rows,
      page: 'report'
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// 6. Break Page (GET) - Shows combined exposure per match+selection against a break limit
app.get('/admin/break', requireAdmin, async (req, res) => {
  const breakLimit = parseFloat(req.query.break_limit) || 100000;

  try {
    // Combine all members' running (PENDING) stakes on the same match + selection
    const exposureRes = await query(`
      SELECT
        m.id as match_id, m.home_team, m.away_team, m.match_time,
        b.bet_type, b.choice,
        COUNT(b.id) as bet_count,
        SUM(b.amount) as total_staked
      FROM bets b
      JOIN matches m ON b.match_id = m.id
      WHERE b.status = 'PENDING' AND m.status IN ('OPEN', 'LIVE')
      GROUP BY m.id, m.home_team, m.away_team, m.match_time, b.bet_type, b.choice
      ORDER BY SUM(b.amount) DESC
    `);

    // Only surface match+selection groups whose combined stake actually exceeds
    // the break limit — this page is a risk alert list, not a full exposure dump.
    const exposureList = exposureRes.rows
      .map(row => {
        const totalStaked = Number(row.total_staked) || 0;
        const excess = totalStaked - breakLimit;
        return {
          ...row,
          total_staked: totalStaked,
          excess,
          isOverLimit: excess > 0
        };
      })
      .filter(e => e.isOverLimit);

    const bettingLocked = await isBettingLocked();

    res.render('admin_break', {
      admin: req.session.user,
      breakLimit,
      exposureList,
      bettingLocked,
      page: 'break'
    });
  } catch (err) {
    return fail(req, res, err, 500);
  }
});

// Toggle Global Bet Lock (Emergency Stop)
app.post('/admin/toggle-bet-lock', requireAdmin, apiLimiter, async (req, res) => {
  try {
    await query(`
      UPDATE app_settings SET betting_locked = NOT betting_locked
      WHERE id = (SELECT id FROM app_settings ORDER BY id LIMIT 1)
    `);
    res.redirect('/admin/break');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// --- Admin Member Actions (POST Routes) ---

// Create Member
app.post('/admin/create-member', requireAdmin, apiLimiter, async (req, res) => {
  const username = cleanString(req.body.username, 50);
  const password = String(req.body.password || '');
  const initialBalance = parseFloat(req.body.initialBalance) || 0;
  const commission = parseFloat(req.body.commission) || 0;
  const bet_limit = parseFloat(req.body.bet_limit) || 100000;

  if (!username || !password || password.length < 6) {
    return res.status(400).json({ success: false, message: 'Username is required and password must be at least 6 characters.' });
  }

  try {
    const passwordHash = await bcrypt.hash(password, 10);
    await query(
      "INSERT INTO users (username, password, role, balance, commission, bet_limit, status) VALUES ($1, $2, 'MEMBER', $3, $4, $5, 'ACTIVE')",
      [username, passwordHash, initialBalance, commission, bet_limit]
    );
    return res.json({ success: true });
  } catch (err) {
    console.error(`[ERROR] ${req.method} ${req.originalUrl}:`, err.message);
    return res.status(400).json({ success: false, message: 'Error creating member. The username may already be taken.' });
  }
});

// Edit Member
app.post('/admin/edit-member/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  const username = cleanString(req.body.username, 50);
  const new_password = String(req.body.new_password || '');
  const balance = parseFloat(req.body.balance) || 0;
  const commission = parseFloat(req.body.commission) || 0;
  const bet_limit = parseFloat(req.body.bet_limit) || 100000;
  const status = (req.body.status === 'SUSPENDED') ? 'SUSPENDED' : 'ACTIVE';

  if (!isValidId(id)) {
    return res.status(400).send('Invalid member id.');
  }
  if (!username) {
    return res.status(400).send('Username is required.');
  }

  try {
    if (new_password && new_password.trim() !== '') {
      if (new_password.length < 6) {
        return res.status(400).send('New password must be at least 6 characters.');
      }
      const passwordHash = await bcrypt.hash(new_password, 10);
      await query(
        "UPDATE users SET username = $1, password = $2, balance = $3, commission = $4, bet_limit = $5, status = $6 WHERE id = $7 AND role = 'MEMBER'",
        [username, passwordHash, balance, commission, bet_limit, status, id]
      );
    } else {
      await query(
        "UPDATE users SET username = $1, balance = $2, commission = $3, bet_limit = $4, status = $5 WHERE id = $6 AND role = 'MEMBER'",
        [username, balance, commission, bet_limit, status, id]
      );
    }
    res.redirect('/admin/members');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// Deposit Credit (adds to member balance, logs to credit_logs)
app.post('/admin/deposit-member/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  const amount = parseFloat(req.body.amount);
  const note = cleanString(req.body.note, 200);

  if (!isValidId(id)) {
    return res.status(400).json({ success: false, message: 'Invalid member id.' });
  }
  if (!amount || isNaN(amount) || amount <= 0) {
    return res.status(400).json({ success: false, message: 'Deposit amount must be a positive number.' });
  }

  try {
    const memberRes = await query("SELECT balance FROM users WHERE id = $1 AND role = 'MEMBER'", [id]);
    const member = memberRes.rows[0];
    if (!member) {
      return res.status(404).json({ success: false, message: 'Member not found.' });
    }

    const newBalance = parseFloat(member.balance) + amount;
    await query('UPDATE users SET balance = $1 WHERE id = $2', [newBalance, id]);
    await query(
      `INSERT INTO credit_logs (user_id, type, amount, balance_after, note, admin_username) VALUES ($1, 'DEPOSIT', $2, $3, $4, $5)`,
      [id, amount, newBalance, note, req.session.user.username]
    );

    return res.json({ success: true, message: 'Deposit successful.', newBalance });
  } catch (err) {
    console.error(`[ERROR] ${req.method} ${req.originalUrl}:`, err.message);
    return res.status(500).json({ success: false, message: 'System error. Please try again.' });
  }
});

// Withdraw Credit (deducts from member balance, logs to credit_logs)
app.post('/admin/withdraw-member/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  const amount = parseFloat(req.body.amount);
  const note = cleanString(req.body.note, 200);

  if (!isValidId(id)) {
    return res.status(400).json({ success: false, message: 'Invalid member id.' });
  }
  if (!amount || isNaN(amount) || amount <= 0) {
    return res.status(400).json({ success: false, message: 'Withdraw amount must be a positive number.' });
  }

  try {
    const memberRes = await query("SELECT balance FROM users WHERE id = $1 AND role = 'MEMBER'", [id]);
    const member = memberRes.rows[0];
    if (!member) {
      return res.status(404).json({ success: false, message: 'Member not found.' });
    }

    const currentBalance = parseFloat(member.balance);
    if (amount > currentBalance) {
      return res.status(400).json({ success: false, message: `Cannot withdraw more than the member's current balance (${currentBalance.toLocaleString()} MMK).` });
    }

    const newBalance = currentBalance - amount;
    await query('UPDATE users SET balance = $1 WHERE id = $2', [newBalance, id]);
    await query(
      `INSERT INTO credit_logs (user_id, type, amount, balance_after, note, admin_username) VALUES ($1, 'WITHDRAW', $2, $3, $4, $5)`,
      [id, amount, newBalance, note, req.session.user.username]
    );

    return res.json({ success: true, message: 'Withdrawal successful.', newBalance });
  } catch (err) {
    console.error(`[ERROR] ${req.method} ${req.originalUrl}:`, err.message);
    return res.status(500).json({ success: false, message: 'System error. Please try again.' });
  }
});

// Toggle Suspend / Active Member
app.post('/admin/toggle-status-member/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  if (!isValidId(id)) {
    return res.status(400).send('Invalid member id.');
  }
  try {
    await query(
      `UPDATE users 
       SET status = CASE WHEN status = 'SUSPENDED' THEN 'ACTIVE' ELSE 'SUSPENDED' END 
       WHERE id = $1 AND role = 'MEMBER'`,
      [id]
    );
    res.redirect('/admin/members');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// Delete Member
app.post('/admin/delete-member/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  if (!isValidId(id)) {
    return res.status(400).send('Invalid member id.');
  }
  try {
    await query("DELETE FROM users WHERE id = $1 AND role = 'MEMBER'", [id]);
    res.redirect('/admin/members');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// --- Admin Match Actions (POST Routes) ---

// Create Football Match
app.post('/admin/add-match', requireAdmin, apiLimiter, async (req, res) => {
  const match_date = cleanString(req.body.match_date, 20);
  const match_time = cleanString(req.body.match_time, 10);
  const home_team = cleanString(req.body.home_team, 100);
  const away_team = cleanString(req.body.away_team, 100);
  const upper_team = (req.body.upper_team === 'AWAY') ? 'AWAY' : 'HOME';
  const body_odd = cleanString(req.body.body_odd, 30);
  const total_odd = cleanString(req.body.total_odd, 30);
  const fullTimestamp = `${match_date} ${match_time}:00`;

  if (!match_date || !match_time || !home_team || !away_team) {
    return res.status(400).send('Please fill in date, time and both team names.');
  }

  try {
    await query(
      `INSERT INTO matches (match_time, home_team, away_team, upper_team, body_odd, total_odd) 
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [fullTimestamp, home_team, away_team, upper_team, body_odd, total_odd]
    );
    res.redirect('/admin/match-list');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// Update Football Match
app.post('/admin/edit-match/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  const match_date = cleanString(req.body.match_date, 20);
  const match_time = cleanString(req.body.match_time, 10);
  const home_team = cleanString(req.body.home_team, 100);
  const away_team = cleanString(req.body.away_team, 100);
  const upper_team = (req.body.upper_team === 'AWAY') ? 'AWAY' : 'HOME';
  const body_odd = cleanString(req.body.body_odd, 30);
  const total_odd = cleanString(req.body.total_odd, 30);
  const status = cleanString(req.body.status, 20);
  const fullTimestamp = `${match_date} ${match_time}:00`;

  if (!isValidId(id)) {
    return res.status(400).send('Invalid match id.');
  }

  try {
    await query(
      `UPDATE matches 
       SET match_time = $1, home_team = $2, away_team = $3, upper_team = $4, body_odd = $5, total_odd = $6, status = $7 
       WHERE id = $8`,
      [fullTimestamp, home_team, away_team, upper_team, body_odd, total_odd, status, id]
    );
    res.redirect('/admin/match-list');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// Delete Football Match
app.post('/admin/delete-match/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  if (!isValidId(id)) {
    return res.status(400).send('Invalid match id.');
  }
  try {
    await query('DELETE FROM matches WHERE id = $1', [id]);
    res.redirect('/admin/match-list');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// =========================================================================
// ADMIN SET RESULT -> AUTO SETTLES BETS (PAYOUT + COMMISSION TO MEMBER BALANCE)
// =========================================================================
app.post('/admin/set-result/:id', requireAdmin, apiLimiter, async (req, res) => {
  const { id } = req.params;
  const hScore = parseInt(req.body.home_score, 10);
  const aScore = parseInt(req.body.away_score, 10);

  if (!isValidId(id)) {
    return res.status(400).send('Invalid match id.');
  }
  if (isNaN(hScore) || isNaN(aScore) || hScore < 0 || aScore < 0) {
    return res.status(400).send('Please enter valid non-negative goal scores.');
  }

  try {
    // 1. Update match score & set status to FINISHED
    await query(
      `UPDATE matches 
       SET home_score = $1, away_score = $2, status = 'FINISHED' 
       WHERE id = $3`,
      [hScore, aScore, id]
    );

    // 2. Fetch match details
    const matchRes = await query('SELECT * FROM matches WHERE id = $1', [id]);
    const match = matchRes.rows[0];

    // 3. Fetch all PENDING bets on this match along with member commission rate
    const pendingBetsRes = await query(`
      SELECT b.*, COALESCE(u.commission, 0) as comm_rate 
      FROM bets b
      JOIN users u ON b.user_id = u.id
      WHERE b.match_id = $1 AND b.status = 'PENDING'
    `, [id]);

    // 4. Calculate Settlement using the external settlement.js module
    for (const bet of pendingBetsRes.rows) {
      const result = settleMyanmarBet(bet, match, hScore, aScore);
      const { status, payout, valid_amount } = result;

      // Update bet status, payout, and valid_amount
      await query(
        'UPDATE bets SET status = $1, payout = $2, valid_amount = $3 WHERE id = $4',
        [status, payout, valid_amount, bet.id]
      );

      // Calculate Commission Amount: (Valid Amount * Commission %)
      const commRate = parseFloat(bet.comm_rate) || 0;
      const commAmount = (parseFloat(valid_amount || 0) * commRate) / 100;

      // Total Refund = Payout + Commission Amount
      const totalRefund = parseFloat(payout || 0) + commAmount;

      if (totalRefund > 0) {
        await query(
          'UPDATE users SET balance = balance + $1 WHERE id = $2',
          [totalRefund, bet.user_id]
        );
      }
    }

    res.redirect('/admin/result');
  } catch (err) {
    return fail(req, res, err, 400);
  }
});

// --- Server Listener ---

const PORT = process.env.PORT || 3001;
Promise.all([ensureSettingsTable(), ensureCreditLogsTable()])
  .then(() => {
    app.listen(PORT, () => {
      console.log(`🚀 Server running on http://localhost:${PORT}`);
    });
  })
  .catch(err => {
    console.error('❌ Failed to initialize app tables:', err.message);
    process.exit(1);
  });
