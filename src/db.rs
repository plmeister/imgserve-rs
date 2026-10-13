use sqlx::SqlitePool;

pub async fn init_db(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS images (
                hash TEXT PRIMARY KEY,
                ext TEXT NOT NULL,
                size INTEGER NOT NULL,
                svg INTEGER NOT NULL DEFAULT 0,
                created_at REAL NOT NULL,
                last_access REAL NOT NULL
            )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;
    Ok(())
}
