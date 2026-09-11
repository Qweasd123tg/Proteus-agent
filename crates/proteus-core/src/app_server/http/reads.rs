use anyhow::Result;
use hyper::StatusCode;
use serde_json::json;

use super::{
    HttpResponse,
    lifecycle::{bootstrap, config_summary_with_activity},
    responses::{error_response, json_response, text_response},
    sessions::{context_map_json, history_json, server_for_query, session_summaries, usage_json},
    sse::sse_response,
    state::HttpAppState,
};
use crate::core::{
    render_topology_map, render_topology_mermaid, render_topology_runtime_mermaid,
    render_topology_runtime_path,
};

pub(super) async fn route_get(
    state: &HttpAppState,
    path: &str,
    query: Option<&str>,
) -> HttpResponse {
    match read(state, path, query).await {
        Ok(response) => response,
        Err(error) => error_response(StatusCode::BAD_REQUEST, &format!("{error:#}")),
    }
}

async fn read(state: &HttpAppState, path: &str, query: Option<&str>) -> Result<HttpResponse> {
    let response = match path {
        "/health" => json_response(StatusCode::OK, &json!({ "ok": true })),
        "/bootstrap" => json_response(StatusCode::OK, &bootstrap(state).await),
        "/sessions" => json_response(StatusCode::OK, &session_summaries(state, None).await?),
        "/history" => json_response(StatusCode::OK, &history_json(state, query).await?),
        "/context" => json_response(StatusCode::OK, &context_map_json(state, query).await?),
        "/usage" => json_response(StatusCode::OK, &usage_json(state, query).await?),
        "/workspace/list" | "/workspace/file" => {
            let server = server_for_query(state, query).await?;
            let root = server.cwd_path().to_path_buf();
            let relative = super::workspace::query_path(query)?;
            let listing = path == "/workspace/list";
            let value = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
                if listing {
                    Ok(serde_json::to_value(super::workspace::list(
                        &root, relative,
                    )?)?)
                } else {
                    Ok(serde_json::to_value(super::workspace::read(
                        &root, relative,
                    )?)?)
                }
            })
            .await??;
            json_response(StatusCode::OK, &value)
        }
        "/events"
        | "/config"
        | "/config/builder"
        | "/model/quota"
        | "/pending"
        | "/sessions/current"
        | "/inspect/topology"
        | "/inspect/plan"
        | "/inspect/topology.mmd"
        | "/inspect/topology.map"
        | "/inspect/topology.runtime"
        | "/inspect/topology.runtime.mmd" => {
            let server = server_for_query(state, query).await?;
            match path {
                "/events" => sse_response(state.clone(), server).await,
                "/config" => json_response(
                    StatusCode::OK,
                    &config_summary_with_activity(state, &server).await?,
                ),
                "/config/builder" => {
                    json_response(StatusCode::OK, &server.config_builder_snapshot().await)
                }
                "/pending" => json_response(StatusCode::OK, &server.pending_requests().await),
                "/model/quota" => match server.model_quota().await {
                    Ok(quota) => json_response(StatusCode::OK, &quota),
                    Err(error) => error_response(StatusCode::BAD_GATEWAY, &format!("{error:#}")),
                },
                "/sessions/current" => json_response(
                    StatusCode::OK,
                    &session_summaries(state, Some(server.cwd_path().to_path_buf())).await?,
                ),
                "/inspect/plan" => json_response(StatusCode::OK, &server.assembly_plan().await),
                path => {
                    let snapshot = server.topology_snapshot().await;
                    match path {
                        "/inspect/topology" => json_response(StatusCode::OK, &snapshot),
                        "/inspect/topology.mmd" => {
                            text_response(StatusCode::OK, render_topology_mermaid(&snapshot))
                        }
                        "/inspect/topology.map" => {
                            text_response(StatusCode::OK, render_topology_map(&snapshot))
                        }
                        "/inspect/topology.runtime" => {
                            text_response(StatusCode::OK, render_topology_runtime_path(&snapshot))
                        }
                        "/inspect/topology.runtime.mmd" => text_response(
                            StatusCode::OK,
                            render_topology_runtime_mermaid(&snapshot),
                        ),
                        _ => unreachable!("validated session read route"),
                    }
                }
            }
        }
        _ => error_response(StatusCode::NOT_FOUND, "unknown app-server HTTP endpoint"),
    };
    Ok(response)
}
