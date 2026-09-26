//! T001: confirms the bundled SQLCipher build ships FTS5, which the derived
//! KB catalog search (research §4) depends on.

#[test]
fn fts5_is_available() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(name, purpose);
         INSERT INTO t(name, purpose) VALUES ('Gaussian blur', 'smooth noise');",
    )
    .expect("FTS5 must be compiled into the bundled SQLite");
    let hits: i64 = db
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'gaussian'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hits, 1);
}
