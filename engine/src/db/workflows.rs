//! `workflows` table queries.

use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

use crate::types::{Workflow, WorkflowGraph, WorkflowSummary};

pub async fn list(db: &PgPool) -> Result<Vec<WorkflowSummary>, sqlx::Error> {
    sqlx::query_as!(
        WorkflowSummary,
        "SELECT id, name, updated_at FROM workflows ORDER BY updated_at DESC, id"
    )
    .fetch_all(db)
    .await
}

pub async fn create(db: &PgPool, name: &str) -> Result<Workflow, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO workflows (name) VALUES ($1)
           RETURNING id, name, graph AS "graph: Json<WorkflowGraph>", created_at, updated_at"#,
        name
    )
    .fetch_one(db)
    .await?;
    Ok(Workflow {
        id: row.id,
        name: row.name,
        graph: row.graph.0,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn get(db: &PgPool, id: Uuid) -> Result<Option<Workflow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT id, name, graph AS "graph: Json<WorkflowGraph>", created_at, updated_at
           FROM workflows WHERE id = $1"#,
        id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| Workflow {
        id: r.id,
        name: r.name,
        graph: r.graph.0,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// Updates only the provided fields (and `updated_at`). Returns `None` if the id does not exist.
pub async fn update(
    db: &PgPool,
    id: Uuid,
    name: Option<&str>,
    graph: Option<&WorkflowGraph>,
) -> Result<Option<Workflow>, sqlx::Error> {
    let graph = graph.map(Json);
    let row = sqlx::query!(
        r#"UPDATE workflows
           SET name = COALESCE($2, name),
               graph = COALESCE($3, graph),
               updated_at = now()
           WHERE id = $1
           RETURNING id, name, graph AS "graph: Json<WorkflowGraph>", created_at, updated_at"#,
        id,
        name,
        graph as Option<Json<&WorkflowGraph>>
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| Workflow {
        id: r.id,
        name: r.name,
        graph: r.graph.0,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// Returns `true` if a row was deleted.
pub async fn delete(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!("DELETE FROM workflows WHERE id = $1", id)
        .execute(db)
        .await?;
    Ok(res.rows_affected() > 0)
}
