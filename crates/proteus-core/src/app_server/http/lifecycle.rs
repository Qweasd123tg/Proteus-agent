use std::path::PathBuf;

use anyhow::{Result, anyhow};
use proteus_contracts::app_protocol::AppBootstrap;
use serde_json::{Value, json};

use super::{
    commands::{cancel_work_for_server, command_response},
    state::HttpAppState,
};
use crate::{
    app_server::{AgentAppServer, AppServerHandle, StdioOutput},
    core::{
        SessionStore, canonicalize_session_dir_path, config_store_root, delete_workspace_session,
        encode_workspace_path,
    },
};

pub(super) async fn bootstrap(state: &HttpAppState) -> AppBootstrap {
    let session_dir = match &state.launch.initial_session_dir {
        Some(path) if state.server_for_session_dir(path).await.is_some() => Some(path.clone()),
        _ => None,
    };
    AppBootstrap {
        session_dir,
        cwd: state.launch.cwd.clone(),
    }
}

pub(super) async fn execute_resume(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    command_response(id, resume_session(state, session_dir).await.map(Some))
}

pub(super) async fn execute_new_session(
    state: &HttpAppState,
    id: Option<String>,
    source_session_dir: Option<PathBuf>,
) -> StdioOutput {
    let result = async {
        let _lifecycle = state.session_lifecycle.lock().await;
        let (config, cwd, config_path) = match source_session_dir {
            Some(path) => {
                let source = super::sessions::server_for_session(state, path).await?;
                let config = source.config.read().await.clone();
                (config, source.cwd.clone(), source.config_path.clone())
            }
            None => (
                state.launch.config.clone(),
                state.launch.cwd.clone(),
                state.launch.config_path.clone(),
            ),
        };
        let next = AgentAppServer::launch(config, cwd, config_path.as_deref()).await?;
        next.start_session().await?;
        state.remember_server(next.clone()).await;
        config_summary_with_activity(state, &next).await
    }
    .await;
    command_response(id, result.map(Some))
}

pub(super) async fn execute_delete_session(
    state: &HttpAppState,
    id: Option<String>,
    session_dir: PathBuf,
) -> StdioOutput {
    let result = async {
        let _lifecycle = state.session_lifecycle.lock().await;
        let session_dir = canonicalize_session_dir_path(session_dir)?;
        let config_path = state
            .launch
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow!("HTTP sessions require a config path for storage"))?;
        let live_server = state.server_for_session_dir(&session_dir).await;
        let was_live = live_server.is_some();
        let workspace = match &live_server {
            Some(server) => server.cwd.clone(),
            None if session_dir.try_exists()? => {
                SessionStore::open(session_dir.clone())?.workspace_path()?
            }
            None => return Ok(Some(json!({ "deleted": false }))),
        };
        // A resumed session may belong to another workspace, but deletion must
        // still stay within this server's configured session store.
        let name = session_dir
            .file_name()
            .ok_or_else(|| anyhow!("invalid session directory"))?;
        let expected = canonicalize_session_dir_path(
            config_store_root(config_path)
                .join("sessions")
                .join(encode_workspace_path(&workspace)?)
                .join(name),
        )?;
        if expected != session_dir {
            return Err(anyhow!(
                "session path is outside configured workspace sessions"
            ));
        }
        if let Some(server) = live_server {
            cancel_work_for_server(state, &server).await;
            server.shutdown().await;
            state.remove_session_server(&session_dir).await;
        }
        let deleted =
            delete_workspace_session(&config_store_root(config_path), &workspace, session_dir)
                .await?;
        Ok(Some(json!({ "deleted": deleted || was_live })))
    }
    .await;
    command_response(id, result)
}

async fn resume_session(state: &HttpAppState, session_dir: PathBuf) -> Result<Value> {
    let _lifecycle = state.session_lifecycle.lock().await;
    let session_dir = canonicalize_session_dir_path(session_dir)?;
    if let Some(existing) = state.server_for_session_dir(&session_dir).await {
        return config_summary_with_activity(state, &existing).await;
    }
    let launch = &state.launch;
    let next = AgentAppServer::launch_resumed(
        launch.config.clone(),
        launch.cwd.clone(),
        launch.config_path.as_deref(),
        session_dir,
    )
    .await?;
    state.remember_server(next.clone()).await;
    config_summary_with_activity(state, &next).await
}

pub(super) async fn config_summary_with_activity(
    state: &HttpAppState,
    server: &AppServerHandle,
) -> Result<Value> {
    let mut summary = server.config_summary().await;
    if let Value::Object(fields) = &mut summary {
        fields.insert(
            "activity".to_owned(),
            serde_json::to_value(state.activity_for_server(server).await)?,
        );
    }
    Ok(summary)
}
