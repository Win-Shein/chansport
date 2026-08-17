// Standardize the whole system's clock on Myanmar Standard Time (UTC+6:30),
// regardless of the host server's own timezone (important for aaPanel deploys).
process.env.TZ = 'Asia/Yangon';

const { Pool } = require('pg');
require('dotenv').config();

const pool = new Pool({
  user: process.env.DB_USER,
  host: process.env.DB_HOST,
  database: process.env.DB_NAME,
  password: process.env.DB_PASSWORD,
  port: process.env.DB_PORT,
  // Set as a Postgres startup parameter (part of the connection handshake itself),
  // so every pooled connection sees Myanmar time for NOW()/CURRENT_TIMESTAMP before
  // any query can run on it — no race with a separate "SET TIME ZONE" query needed.
  options: '-c TimeZone=Asia/Yangon',
});

pool.on('connect', () => {
  console.log('✅ PostgreSQL Database ချိတ်ဆက်မှု အောင်မြင်ပါသည်။');
});

module.exports = {
  query: (text, params) => pool.query(text, params),
  pool,
};