use sqlx::SqlitePool;
use chrono::Utc;

pub async fn init_db(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS images (
                hash TEXT PRIMARY KEY,
                mimetype TEXT NOT NULL,
                ext TEXT NOT NULL,
                size INTEGER NOT NULL,
                created_at REAL NOT NULL,
                last_access REAL NOT NULL
            )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;
    Ok(())
}

pub async fn upsert_image(
    pool: &SqlitePool,
    hash: &str,
    mimetype: &str,
    ext: &str,
    size: i64,
) -> anyhow::Result<()> {
    let now = Utc::now().timestamp() as f64;
    sqlx::query(
        "INSERT OR REPLACE INTO images (hash, mimetype, ext, size, created_at, last_access) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(hash)
    .bind(mimetype)
    .bind(ext)
    .bind(size)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch_last_access(pool: &SqlitePool, hash: &str) -> anyhow::Result<()> {
    let now = Utc::now().timestamp() as f64;
    sqlx::query("UPDATE images SET last_access = ? WHERE hash = ?")
        .bind(now)
        .bind(hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_image(pool: &SqlitePool, hash: &str) -> anyhow::Result<Option<(String, String)>> {
    let row = sqlx::query_as::<_, (String, String)>("SELECT mimetype, ext FROM images WHERE hash = ?")
        .bind(hash)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}
