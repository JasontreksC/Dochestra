//! `bundles` / `bundle_files` queries.

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{Bundle, BundleFile, BundleSummary};

/// A validated upload ready to be recorded.
pub struct NewBundleFile {
    pub id: Uuid,
    pub original_name: String,
    pub stored_path: String,
    pub size_bytes: i64,
}

pub async fn list(db: &PgPool) -> Result<Vec<BundleSummary>, sqlx::Error> {
    sqlx::query_as!(
        BundleSummary,
        r#"SELECT b.id, b.name, COUNT(f.id) AS "file_count!", b.created_at
           FROM bundles b LEFT JOIN bundle_files f ON f.bundle_id = b.id
           GROUP BY b.id
           ORDER BY b.created_at DESC, b.id"#
    )
    .fetch_all(db)
    .await
}

pub async fn create(db: &PgPool, name: &str) -> Result<Bundle, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO bundles (name) VALUES ($1) RETURNING id, name, created_at",
        name
    )
    .fetch_one(db)
    .await?;
    Ok(Bundle {
        id: row.id,
        name: row.name,
        created_at: row.created_at,
        files: vec![],
    })
}

pub async fn get(db: &PgPool, id: Uuid) -> Result<Option<Bundle>, sqlx::Error> {
    let Some(row) = sqlx::query!("SELECT id, name, created_at FROM bundles WHERE id = $1", id)
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };
    let files = sqlx::query_as!(
        BundleFile,
        "SELECT id, original_name, size_bytes, position, created_at
         FROM bundle_files WHERE bundle_id = $1 ORDER BY position",
        id
    )
    .fetch_all(db)
    .await?;
    Ok(Some(Bundle {
        id: row.id,
        name: row.name,
        created_at: row.created_at,
        files,
    }))
}

pub async fn exists(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM bundles WHERE id = $1) AS "exists!""#,
        id
    )
    .fetch_one(db)
    .await
}

/// Returns `true` if the bundle existed (its files are removed by cascade).
pub async fn delete(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!("DELETE FROM bundles WHERE id = $1", id)
        .execute(db)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Appends files after the current last position, atomically. Returns `None` if the bundle
/// does not exist (e.g. it was deleted during the upload).
pub async fn add_files(
    db: &PgPool,
    bundle_id: Uuid,
    files: &[NewBundleFile],
) -> Result<Option<Vec<BundleFile>>, sqlx::Error> {
    let mut tx = db.begin().await?;
    // The row lock serializes concurrent uploads to the same bundle (position assignment).
    let locked = sqlx::query_scalar!("SELECT id FROM bundles WHERE id = $1 FOR UPDATE", bundle_id)
        .fetch_optional(&mut *tx)
        .await?;
    if locked.is_none() {
        return Ok(None);
    }
    let mut next = sqlx::query_scalar!(
        r#"SELECT COALESCE(MAX(position), -1) + 1 AS "next!" FROM bundle_files WHERE bundle_id = $1"#,
        bundle_id
    )
    .fetch_one(&mut *tx)
    .await?;

    let mut created = Vec::with_capacity(files.len());
    for file in files {
        let row = sqlx::query_as!(
            BundleFile,
            "INSERT INTO bundle_files (id, bundle_id, original_name, stored_path, size_bytes, position)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, original_name, size_bytes, position, created_at",
            file.id,
            bundle_id,
            file.original_name,
            file.stored_path,
            file.size_bytes,
            next
        )
        .fetch_one(&mut *tx)
        .await?;
        created.push(row);
        next += 1;
    }
    tx.commit().await?;
    Ok(Some(created))
}

/// Deletes one file row and returns its stored path, or `None` if it does not exist.
pub async fn delete_file(
    db: &PgPool,
    bundle_id: Uuid,
    file_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM bundle_files WHERE bundle_id = $1 AND id = $2 RETURNING stored_path",
        bundle_id,
        file_id
    )
    .fetch_optional(db)
    .await
}

/// A bundle file as needed by the `bundle` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub original_name: String,
    pub stored_path: String,
}

/// Files of a bundle in position order, or `None` if the bundle does not exist.
pub async fn stored_files(db: &PgPool, id: Uuid) -> Result<Option<Vec<StoredFile>>, sqlx::Error> {
    if !exists(db, id).await? {
        return Ok(None);
    }
    let files = sqlx::query_as!(
        StoredFile,
        "SELECT original_name, stored_path FROM bundle_files WHERE bundle_id = $1 ORDER BY position",
        id
    )
    .fetch_all(db)
    .await?;
    Ok(Some(files))
}
