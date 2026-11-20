//! Forward-only schema migrations tracked with `PRAGMA user_version`.

use rusqlite::{Connection, TransactionBehavior};

use crate::Error;

/// Migrations of `user.db`, in order. Never edit a shipped entry: append.
pub(crate) const USER: &[&str] = &[include_str!("../migrations/user/0001_training.sql")];

/// Bring `conn` to the latest of `migrations`. Each step runs in its own
/// transaction and must leave no foreign-key violation behind.
pub(crate) fn run(conn: &mut Connection, migrations: &[&str]) -> Result<(), Error> {
    let supported = migrations.len();
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let current = usize::try_from(current).unwrap_or(usize::MAX);
    if current > supported {
        return Err(Error::NewerSchema {
            found: current,
            supported,
        });
    }
    for (index, sql) in migrations.iter().enumerate().skip(current) {
        let version = index + 1;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        let violations: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if violations > 0 {
            return Err(Error::MigrationIntegrity { version });
        }
        tx.pragma_update(
            None,
            "user_version",
            u32::try_from(version).unwrap_or(u32::MAX),
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::run;
    use crate::Error;

    const STEPS: &[&str] = &[
        "CREATE TABLE a (id INTEGER PRIMARY KEY) STRICT;",
        "CREATE TABLE b (a_id INTEGER REFERENCES a (id)) STRICT;",
    ];

    fn version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap_or(u32::MAX)
    }

    #[test]
    fn applies_pending_steps_once() -> Result<(), Error> {
        let mut conn = Connection::open_in_memory()?;
        run(&mut conn, &STEPS[..1])?;
        assert_eq!(version(&conn), 1);
        run(&mut conn, STEPS)?;
        run(&mut conn, STEPS)?;
        assert_eq!(version(&conn), 2);
        Ok(())
    }

    #[test]
    fn refuses_newer_files() -> Result<(), Error> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "user_version", 9)?;
        assert!(matches!(
            run(&mut conn, STEPS),
            Err(Error::NewerSchema {
                found: 9,
                supported: 2
            })
        ));
        Ok(())
    }

    #[test]
    fn rolls_back_steps_that_break_foreign_keys() -> Result<(), Error> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", false)?;
        let broken = ["CREATE TABLE a (id INTEGER PRIMARY KEY) STRICT;
             CREATE TABLE b (a_id INTEGER REFERENCES a (id)) STRICT;
             INSERT INTO b VALUES (42);"];
        assert!(matches!(
            run(&mut conn, &broken),
            Err(Error::MigrationIntegrity { version: 1 })
        ));
        assert_eq!(version(&conn), 0);
        Ok(())
    }
}
