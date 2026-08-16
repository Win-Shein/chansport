const { pool, query } = require('./db');
const bcrypt = require('bcryptjs');
require('dotenv').config();

async function resetDatabase() {
  try {
    console.log('🔄 Database အဟောင်းများကို ရှင်းထုတ်နေပါသည်...');

    // ၁။ Table အဟောင်းများအားလုံးကို ဖျက်ချခြင်း
    await query('DROP TABLE IF EXISTS bets CASCADE;');
    await query('DROP TABLE IF EXISTS matches CASCADE;');
    await query('DROP TABLE IF EXISTS users CASCADE;');

    console.log('✅ Table အဟောင်းများ ရှင်းထုတ်ပြီးပါပြီ။');
    console.log('🛠 Table အသစ်များ တည်ဆောက်နေပါသည်...');

    // ၂။ Users Table (bet_limit နှင့် commission ပါဝင်ပြီး)
    await query(`
      CREATE TABLE users (
        id SERIAL PRIMARY KEY,
        username VARCHAR(50) UNIQUE NOT NULL,
        password VARCHAR(255) NOT NULL,
        role VARCHAR(20) DEFAULT 'MEMBER',
        balance NUMERIC(15, 2) DEFAULT 0.00,
        commission NUMERIC(5, 2) DEFAULT 0.00,
        bet_limit NUMERIC(15, 2) DEFAULT 100000.00,
        status VARCHAR(20) DEFAULT 'ACTIVE',
        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      );
    `);

    // ၃။ Matches Table
    await query(`
      CREATE TABLE matches (
        id SERIAL PRIMARY KEY,
        match_time TIMESTAMP NOT NULL,
        home_team VARCHAR(100) NOT NULL,
        away_team VARCHAR(100) NOT NULL,
        upper_team VARCHAR(10) NOT NULL DEFAULT 'HOME',
        body_odd VARCHAR(50) NOT NULL DEFAULT '0-0',
        total_odd VARCHAR(50) NOT NULL DEFAULT '2.5-0',
        home_score INT DEFAULT NULL,
        away_score INT DEFAULT NULL,
        status VARCHAR(20) DEFAULT 'OPEN',
        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      );
    `);

    // ၄။ Bets Table (valid_amount ပါဝင်ပြီး)
    await query(`
      CREATE TABLE bets (
        id SERIAL PRIMARY KEY,
        user_id INT REFERENCES users(id) ON DELETE CASCADE,
        match_id INT REFERENCES matches(id) ON DELETE CASCADE,
        bet_type VARCHAR(30) NOT NULL,
        choice VARCHAR(100) NOT NULL,
        odd VARCHAR(50) NOT NULL,
        amount NUMERIC(15, 2) NOT NULL,
        payout NUMERIC(15, 2) DEFAULT 0.00,
        valid_amount NUMERIC(15, 2) DEFAULT 0.00,
        status VARCHAR(20) DEFAULT 'PENDING',
        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      );
    `);

    // ၅။ App Settings Table (Global Bet Lock) - kept across resets, not dropped above
    await query(`
      CREATE TABLE IF NOT EXISTS app_settings (
        id SERIAL PRIMARY KEY,
        betting_locked BOOLEAN NOT NULL DEFAULT FALSE
      );
    `);
    const existingSettings = await query('SELECT id FROM app_settings LIMIT 1');
    if (existingSettings.rows.length === 0) {
      await query('INSERT INTO app_settings (betting_locked) VALUES (FALSE)');
    }

    console.log('✅ Table အသစ်များ အောင်မြင်စွာ တည်ဆောက်ပြီးပါပြီ။');
    console.log('👤 Default Admin နှင့် Member စာရင်း သွင်းနေပါသည်...');

        // ၆။ မူလ Admin Account (admin / admin123) - password hashed with bcrypt
    const adminHash = await bcrypt.hash('admin123', 10);
    await query(`
      INSERT INTO users (username, password, role, balance, commission, bet_limit, status)
      VALUES ('admin', $1, 'ADMIN', 0.00, 0.00, 0.00, 'ACTIVE');
    `, [adminHash]);

    // ၇။ Demo Member Account (win / 123456) - password hashed with bcrypt
    const winHash = await bcrypt.hash('123456', 10);
    await query(`
      INSERT INTO users (username, password, role, balance, commission, bet_limit, status)
      VALUES ('win', $1, 'MEMBER', 100000.00, 5.00, 50000.00, 'ACTIVE');
    `, [winHash]);

    console.log('🎉 Database Clean Reset အောင်မြင်စွာ ပြီးဆုံးပါပြီ!');
    console.log('------------------------------------------------------------');
    console.log('🔑 Admin Account  : username = admin | password = admin123');
    console.log('👤 Member Account : username = win   | password = 123456');
    console.log('------------------------------------------------------------');

  } catch (err) {
    console.error('❌ Database Reset Error:', err.message);
  } finally {
    await pool.end();
    process.exit();
  }
}

resetDatabase();