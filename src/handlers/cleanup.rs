use std::fs;
use sqlx::Row;

use crate::config::AppState;
use crate::util::now_secs;

pub async fn cleanup(state: &AppState) -> anyhow::Result<()> {
    let now = now_secs();
    let ttl_days = state.args.ttl_days;
    if ttl_days > 0.0 {
        let cutoff = now - ttl_days * 86400.0;
        let rows = sqlx::query("SELECT hash, ext FROM images WHERE created_at < ?")
            .bind(cutoff)
            .fetch_all(&state.pool)
            .await?;
        for row in rows {
            let hash: String = row.get("hash");
            let ext: String = row.get("ext");
            let _ = delete_image(state, &hash, &ext).await;
        }
    }
    let quota_mb = state.args.quota_mb;
    if quota_mb > 0.0 {
        let quota_bytes = (quota_mb * 1024.0 * 1024.0) as i64;
        loop {
            let total: i64 = sqlx::query("SELECT COALESCE(SUM(size), 0) FROM images")
                .fetch_one(&state.pool)
                .await?
                .get(0);
            if total <= quota_bytes {
                break;
            }
            let oldest = sqlx::query("SELECT hash, ext FROM images ORDER BY last_access ASC LIMIT 1")
                .fetch_optional(&state.pool)
                .await?;
            match oldest {
                Some(row) => {
                    let hash: String = row.get("hash");
                    let ext: String = row.get("ext");
                    let _ = delete_image(state, &hash, &ext).await;
                }
                None => break,
            }
        }
    }
    Ok(())
}

async fn delete_image(state: &AppState, hash: &str, ext: &str) -> anyhow::Result<()> {
    let mut paths = Vec::new();
    if ext == ".svg" {
        paths.push(state.orig_dir.join(format!("{hash}{ext}.gz")));
    }
    paths.push(state.orig_dir.join(format!("{hash}{ext}")));
    for p in paths {
        let _ = fs::remove_file(p);
    }
    sqlx::query("DELETE FROM images WHERE hash = ?")
        .bind(hash)
        .execute(&state.pool)
        .await?;
    Ok(())
}
