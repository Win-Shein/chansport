CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT UNIQUE NOT NULL,
  password TEXT NOT NULL,
  role TEXT DEFAULT 'MEMBER',
  balance REAL DEFAULT 0,
  commission REAL DEFAULT 0,
  bet_limit REAL DEFAULT 100000,
  status TEXT DEFAULT 'ACTIVE',
  created_at TEXT DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now','+6 hours','+30 minutes'))
);

CREATE TABLE IF NOT EXISTS matches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  match_time TEXT NOT NULL,
  home_team TEXT NOT NULL,
  away_team TEXT NOT NULL,
  upper_team TEXT NOT NULL DEFAULT 'HOME',
  body_odd TEXT NOT NULL DEFAULT '0-0',
  total_odd TEXT NOT NULL DEFAULT '2.5-0',
  home_score INTEGER,
  away_score INTEGER,
  status TEXT DEFAULT 'OPEN',
  created_at TEXT DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now','+6 hours','+30 minutes'))
);

CREATE TABLE IF NOT EXISTS bets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
  match_id INTEGER REFERENCES matches(id) ON DELETE CASCADE,
  bet_type TEXT NOT NULL,
  choice TEXT NOT NULL,
  odd TEXT NOT NULL,
  amount REAL NOT NULL,
  payout REAL DEFAULT 0,
  valid_amount REAL DEFAULT 0,
  status TEXT DEFAULT 'PENDING',
  created_at TEXT DEFAULT (strftime('%Y-%m-%d %H:%M:%S','now','+6 hours','+30 minutes'))
);

CREATE TABLE IF NOT EXISTS app_settings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  betting_locked INTEGER NOT NULL DEFAULT 0
);
