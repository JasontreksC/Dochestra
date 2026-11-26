//! Workflow CRUD endpoints (CONTEXT §4.1.3).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use super::extract::{ApiJson, ApiPath};
use crate::db;
use crate::error::ApiError;
use crate::graph::planner::plan_run;
use crate::graph::validator::validate_structure;
use crate::state::AppState;
use crate::types::{
    CreateWorkflowRequest, UpdateWorkflowRequest, ValidationReport, Workflow, WorkflowSummary,
};

const NAME_MAX_CHARS: usize = 100;

/// Trims the name and enforces 1..=100 characters.
pub(super) fn normalize_name(raw: &str) -> Result<&str, ApiError> {
    let name = raw.trim();
    let len = name.chars().count();
    if len == 0 || len > NAME_MAX_CHARS {
        return Err(ApiError::BadRequest(format!(
            "name must be 1-{NAME_MAX_CHARS} characters"
        )));
    }
    Ok(name)
}

fn not_found(id: Uuid) -> ApiError {
    ApiError::NotFound(format!("workflow {id} not found"))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<WorkflowSummary>>, ApiError> {
    Ok(Json(db::workflows::list(&state.db).await?))
}

pub async fn create(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateWorkflowRequest>,
) -> Result<(StatusCode, Json<Workflow>), ApiError> {
    let name = normalize_name(&req.name)?;
    let wf = db::workflows::create(&state.db, name).await?;
    Ok((StatusCode::CREATED, Json(wf)))
}

pub async fn get(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Workflow>, ApiError> {
    db::workflows::get(&state.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| not_found(id))
}

pub async fn update(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateWorkflowRequest>,
) -> Result<Json<Workflow>, ApiError> {
    let name = req.name.as_deref().map(normalize_name).transpose()?;
    if let Some(graph) = &req.graph {
        let issues = validate_structure(graph);
        if !issues.is_empty() {
            return Err(ApiError::Validation {
                message: "그래프 구조 검증 실패".to_owned(),
                issues,
            });
        }
    }
    db::workflows::update(&state.db, id, name, req.graph.as_ref())
        .await?
        .map(Json)
        .ok_or_else(|| not_found(id))
}

pub async fn delete(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    if db::workflows::delete(&state.db, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found(id))
    }
}

/// Runs the full pre-run validation on the stored graph without executing it.
pub async fn validate(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ValidationReport>, ApiError> {
    let wf = db::workflows::get(&state.db, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    let report = match plan_run(&wf.graph) {
        Ok(_) => ValidationReport {
            ok: true,
            issues: vec![],
        },
        Err(issues) => ValidationReport { ok: false, issues },
    };
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules() {
        assert_eq!(normalize_name("  ok  ").unwrap(), "ok");
        assert!(normalize_name("   ").is_err());
        assert!(normalize_name(&"x".repeat(101)).is_err());
        assert!(normalize_name(&"가".repeat(100)).is_ok());
    }
}
