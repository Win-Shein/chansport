const express = require('express');
const path = require('path');
const session = require('express-session');
const { pool, query } = require('./db');
const { settleMyanmarBet } = require('./settlement');
require('dotenv').config();

const app = express();

// View Engine Setup (EJS)
app.set('view engine', 'ejs');
app.set('views', path.join(__dirname, 'views'));

// Body Parsers
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Session Setup
app.use(session({
  secret: process.env.SESSION_SECRET || 'myanbet365_secret_key',
  resave: false,
  saveUninitialized: false,
  cookie: { 
    maxAge: 24 * 60 * 60 * 1000 // 1 day
  }
}));

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
app.post('/login', async (req, res) => {
  const { username, password } = req.body;
  try {
    const result = await query(
      'SELECT * FROM users WHERE username = $1 AND password = $2',
      [username, password]
    );

    if (result.rows.length === 0) {
      return res.render('login', { error: 'Invalid username or password' });
    }

    const user = result.rows[0];

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
    res.render('login', { error: err.message });
  }
});

// Logout (GET)
app.get('/logout', (req, res) => {
  req.session.destroy(() => {
    res.redirect('/login');
  });
});

// --- Member Routes ---

// Member Dashboard (GET) - Tabs: sports, history, winlose
app.get('/member', requireAuth, async (req, res) => {
  const currentTab = req.query.tab || 'sports';
  const { filter_date } = req.query;
  const userId = req.session.user.id;

  try {
    const userRes = await query('SELECT * FROM users WHERE id = $1', [userId]);

    // 1. Active open/live matches
    const matchesRes = await query(`
      SELECT * FROM matches 
      WHERE status IN ('OPEN', 'LIVE') 
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
      winLoseQuery += ` AND DATE(b.created_at) = $2`;
      winLoseParams.push(filter_date);
    }
    winLoseQuery += ` ORDER BY b.created_at DESC`;

    const winLoseRes = await query(winLoseQuery, winLoseParams);

    // Calculate Summary Metrics
    const totalStake = winLoseRes.rows.reduce((sum, r) => sum + Number(r.amount || 0), 0);
    const totalPayout = winLoseRes.rows.reduce((sum, r) => sum + Number(r.payout || 0), 0);
    const netWinLose = totalPayout - totalStake;

    res.render('member_dashboard', {
      user: userRes.rows[0],
      currentTab,
      filter_date: filter_date || '',
      matches: matchesRes.rows,
      runningBets: runningBetsRes.rows,
      winLoseBets: winLoseRes.rows,
      totalStake,
      totalPayout,
      netWinLose
    });
  } catch (err) {
    res.status(500).send(err.message);
  }
});

// Member Place Bet (POST) - Returns JSON for smooth AJAX interaction
app.post('/member/bet', requireAuth, async (req, res) => {
  const { match_id, bet_type, choice, odd, amount } = req.body;
  const userId = req.session.user.id;
  const betAmount = parseFloat(amount);

  if (!betAmount || betAmount <= 0) {
    return res.status(400).json({ success: false, message: 'လောင်းကြေး ပမာဏ မှားယွင်းနေပါသည်။' });
  }

  try {
    const userRes = await query(
      'SELECT balance, status, COALESCE(bet_limit, 100000) as bet_limit FROM users WHERE id = $1',
      [userId]
    );
    const user = userRes.rows[0];

    if (user.status === 'SUSPENDED') {
      return res.status(403).json({ success: false, message: 'သင့်အကောင့်ကို ပိတ်ထားပါသဖြင့် လောင်း၍မရနိုင်ပါ။' });
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
    return res.status(500).json({ success: false, message: `System error: ${err.message}` });
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
    res.status(500).send(err.message);
  }
});

// 2. Add Match Page (GET)
app.get('/admin/add-match', requireAdmin, (req, res) => {
  res.render('admin_add_match', {
    admin: req.session.user,
    page: 'add_match'
  });
});

// 3. Match List Page (GET)
app.get('/admin/match-list', requireAdmin, async (req, res) => {
  const { filter_date } = req.query;

  try {
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
      sqlQuery = `
        SELECT * FROM matches 
        WHERE DATE(match_time) = $1
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
      filter_date: filter_date || '',
      page: 'match_list'
    });
  } catch (err) {
    res.status(500).send(err.message);
  }
});

// 4. Result Page (GET)
app.get('/admin/result', requireAdmin, async (req, res) => {
  const { filter_date } = req.query;

  try {
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
      sqlQuery = `
        SELECT * FROM matches 
        WHERE DATE(match_time) = $1
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
      filter_date: filter_date || '',
      page: 'result'
    });
  } catch (err) {
    res.status(500).send(err.message);
  }
});

// 5. Report Page (GET) - Credit, Outstanding, Win/Lose with Valid Amount & Commission
app.get('/admin/report', requireAdmin, async (req, res) => {
  const subTab = req.query.tab || 'credit';
  const { filter_date } = req.query;

  try {
    // 1. Credit Data
    const creditMembersRes = await query(
      "SELECT id, username, balance, COALESCE(commission, 0) as commission, COALESCE(bet_limit, 100000) as bet_limit, COALESCE(status, 'ACTIVE') as status, created_at FROM users WHERE role = 'MEMBER' ORDER BY balance DESC"
    );

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
    let detailedBetsQuery = `
      SELECT b.*, u.username, m.home_team, m.away_team, m.home_score, m.away_score, m.match_time
      FROM bets b
      JOIN users u ON b.user_id = u.id
      JOIN matches m ON b.match_id = m.id
      WHERE b.status IN ('WON', 'LOST', 'DRAW')
      ${filter_date ? 'AND DATE(b.created_at) = $1' : ''}
      ORDER BY b.created_at DESC
    `;
    const detailedBetsRes = await query(detailedBetsQuery, filter_date ? [filter_date] : []);

    // 4. Win/Lose Aggregated Member Data by Valid Amount
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
      ${filter_date ? 'AND DATE(b.created_at) = $1' : ''}
      WHERE u.role = 'MEMBER'
      GROUP BY u.id, u.username, u.commission
      ORDER BY turnover DESC
    `;
    let winLoseParams = filter_date ? [filter_date] : [];
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

    res.render('admin_report', {
      admin: req.session.user,
      subTab,
      filter_date: filter_date || '',
      creditMembers: creditMembersRes.rows,
      outstandingBets: outstandingRes.rows,
      winLoseList,
      allSettledBets: detailedBetsRes.rows,
      page: 'report'
    });
  } catch (err) {
    res.status(500).send(err.message);
  }
});

// 6. Break Page (GET)
app.get('/admin/break', requireAdmin, (req, res) => {
  res.render('admin_break', {
    admin: req.session.user,
    page: 'break'
  });
});

// --- Admin Member Actions (POST Routes) ---

// Create Member
app.post('/admin/create-member', requireAdmin, async (req, res) => {
  const { username, password, initialBalance, commission, bet_limit } = req.body;
  try {
    await query(
      "INSERT INTO users (username, password, role, balance, commission, bet_limit, status) VALUES ($1, $2, 'MEMBER', $3, $4, $5, 'ACTIVE')",
      [username, password, initialBalance || 0, commission || 0, bet_limit || 100000]
    );
    res.redirect('/admin/members');
  } catch (err) {
    res.status(400).send(`Error creating member: ${err.message}`);
  }
});

// Edit Member
app.post('/admin/edit-member/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  const { username, new_password, balance, commission, bet_limit, status } = req.body;

  try {
    if (new_password && new_password.trim() !== '') {
      await query(
        "UPDATE users SET username = $1, password = $2, balance = $3, commission = $4, bet_limit = $5, status = $6 WHERE id = $7 AND role = 'MEMBER'",
        [username, new_password, balance, commission || 0, bet_limit || 100000, status, id]
      );
    } else {
      await query(
        "UPDATE users SET username = $1, balance = $2, commission = $3, bet_limit = $4, status = $5 WHERE id = $6 AND role = 'MEMBER'",
        [username, balance, commission || 0, bet_limit || 100000, status, id]
      );
    }
    res.redirect('/admin/members');
  } catch (err) {
    res.status(400).send(`Error updating member: ${err.message}`);
  }
});

// Toggle Suspend / Active Member
app.post('/admin/toggle-status-member/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  try {
    await query(
      `UPDATE users 
       SET status = CASE WHEN status = 'SUSPENDED' THEN 'ACTIVE' ELSE 'SUSPENDED' END 
       WHERE id = $1 AND role = 'MEMBER'`,
      [id]
    );
    res.redirect('/admin/members');
  } catch (err) {
    res.status(400).send(`Error changing status: ${err.message}`);
  }
});

// Delete Member
app.post('/admin/delete-member/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  try {
    await query("DELETE FROM users WHERE id = $1 AND role = 'MEMBER'", [id]);
    res.redirect('/admin/members');
  } catch (err) {
    res.status(400).send(`Error deleting member: ${err.message}`);
  }
});

// --- Admin Match Actions (POST Routes) ---

// Create Football Match
app.post('/admin/add-match', requireAdmin, async (req, res) => {
  const { match_date, match_time, home_team, away_team, upper_team, body_odd, total_odd } = req.body;
  const fullTimestamp = `${match_date} ${match_time}:00`;

  try {
    await query(
      `INSERT INTO matches (match_time, home_team, away_team, upper_team, body_odd, total_odd) 
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [fullTimestamp, home_team, away_team, upper_team, body_odd, total_odd]
    );
    res.redirect('/admin/match-list');
  } catch (err) {
    res.status(400).send(`Error adding match: ${err.message}`);
  }
});

// Update Football Match
app.post('/admin/edit-match/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  const { match_date, match_time, home_team, away_team, upper_team, body_odd, total_odd, status } = req.body;
  const fullTimestamp = `${match_date} ${match_time}:00`;

  try {
    await query(
      `UPDATE matches 
       SET match_time = $1, home_team = $2, away_team = $3, upper_team = $4, body_odd = $5, total_odd = $6, status = $7 
       WHERE id = $8`,
      [fullTimestamp, home_team, away_team, upper_team, body_odd, total_odd, status, id]
    );
    res.redirect('/admin/match-list');
  } catch (err) {
    res.status(400).send(`Error updating match: ${err.message}`);
  }
});

// Delete Football Match
app.post('/admin/delete-match/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  try {
    await query('DELETE FROM matches WHERE id = $1', [id]);
    res.redirect('/admin/match-list');
  } catch (err) {
    res.status(400).send(`Error deleting match: ${err.message}`);
  }
});

// =========================================================================
// ADMIN SET RESULT -> AUTO SETTLES BETS (PAYOUT + COMMISSION TO MEMBER BALANCE)
// =========================================================================
app.post('/admin/set-result/:id', requireAdmin, async (req, res) => {
  const { id } = req.params;
  const { home_score, away_score } = req.body;
  const hScore = parseInt(home_score, 10);
  const aScore = parseInt(away_score, 10);

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
      const { status, payout, valid_amount } = settleMyanmarBet(bet, match, hScore, aScore);

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
    res.status(400).send(`Error saving result and settling bets: ${err.message}`);
  }
});

// --- Server Listener ---

const PORT = process.env.PORT || 3000;
app.listen(PORT, () => {
  console.log(`🚀 Server running on http://localhost:${PORT}`);
});