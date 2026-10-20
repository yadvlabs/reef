//! Expand and contract: a database expanded by a newer build (within `SCHEMA_RANGE`) keeps
//! working, one beyond the range is refused, and a migration that was applied with different
//! contents (another build's file) is refused with a message naming the fix.

use reef_db::{Db, SCHEMA_RANGE, latest_migration};

fn temp_url(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("reef-schema-{tag}-{}", reef_core::token::random_hex(6)));
    format!("sqlite://{}/reef.db", dir.display())
}

async fn insert_applied(db: &Db, version: i64, checksum: &[u8]) {
    sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES ($1, $2, TRUE, $3, 0)")
        .bind(version)
        .bind(format!("fake {version}"))
        .bind(checksum.to_vec())
        .execute(db.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn tolerates_an_expansion_by_a_newer_build_within_the_range() {
    let url = temp_url("expanded");
    let db = Db::connect(&url).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), latest_migration());
    assert!(SCHEMA_RANGE.contains(latest_migration()), "the build runs against the schema it migrates to");
    assert_eq!(SCHEMA_RANGE.max, latest_migration() + 1, "binary N must run against schema N+1");
    // A newer build applied the next migration (schema N+1): this build keeps running.
    insert_applied(&db, SCHEMA_RANGE.max, b"future").await;
    let db = Db::connect(&url).await.expect("schema N+1 is supported");
    assert_eq!(db.schema_version().await.unwrap(), SCHEMA_RANGE.max);
    assert_eq!(db.check_schema(SCHEMA_RANGE).await.unwrap(), SCHEMA_RANGE.max);
    // One more than the range: refused.
    insert_applied(&db, SCHEMA_RANGE.max + 1, b"further").await;
    let err = Db::connect(&url).await.expect_err("beyond the range is refused").to_string();
    assert!(err.contains("newer than this build supports"), "{err}");
}

#[tokio::test]
async fn refuses_a_migration_applied_with_different_contents() {
    let url = temp_url("mismatch");
    let db = Db::connect_without_migrating(&url).await.unwrap();
    sqlx::raw_sql("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL)")
        .execute(db.pool())
        .await
        .unwrap();
    // Version 1 applied from a file with other contents: refused, naming the fix.
    insert_applied(&db, 1, &[7u8; 48]).await;
    let err = Db::connect(&url).await.expect_err("another build's migration is refused").to_string();
    assert!(err.contains("different build") && err.contains("make reset"), "{err}");
    let err = db.migrate().await.expect_err("migrate refuses it too").to_string();
    assert!(err.contains("different build"), "{err}");
}
