// ============================================================
// PASSWORD MIGRATION SCRIPT
// Converts any legacy plaintext passwords in the `users` table
// into secure bcrypt hashes. Existing hashed passwords are left
// untouched. Safe to run multiple times.
//
// Run:  node migrate_passwords.js
// ============================================================
const bcrypt = require('bcryptjs');
const { pool, query } = require('./db');
require('dotenv').config();

(async () => {
  try {
    const res = await query('SELECT id, username, password FROM users');
    let updated = 0;

    for (const user of res.rows) {
      const pwd = String(user.password);

      // bcrypt hashes always start with "$2a$", "$2b$" or "$2y$"
      const isAlreadyHashed = /^\$2[aby]\$/.test(pwd);

      if (!isAlreadyHashed) {
        // Legacy plaintext -> hash it
        const hash = await bcrypt.hash(pwd, 10);
        await query('UPDATE users SET password = $1 WHERE id = $2', [hash, user.id]);
        updated++;
        console.log(`  migrated '${user.username}' (id=${user.id})`);
      }
    }

    console.log(`\nDone. Migrated ${updated} password(s). Others already hashed.`);
  } catch (err) {
    console.error('Migration failed:', err.message);
    process.exitCode = 1;
  } finally {
    await pool.end();
    process.exit();
  }
})();
