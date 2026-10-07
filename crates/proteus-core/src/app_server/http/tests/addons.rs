use super::*;
use proteus_contracts::app_protocol::addons::{AppAddonsSnapshot, AppAddonsUpdate};

fn skill(root: &std::path::Path, name: &str, body: &str) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    std::fs::write(
        root.join(name).join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\n---\n{body}\n"),
    )
    .unwrap();
}

async fn load_skill(server: &AppServerHandle, name: &str) -> crate::domain::ToolResult {
    server
        .runtime
        .execute_tool(
            ToolCall::new(new_call_id(), "skill", json!({"name":name})),
            CancellationToken::new(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn addons_http_manages_real_skills_plugins_and_mcp_without_a_model_or_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let configs = tempfile::tempdir().unwrap();
    let config_path = configs.path().join("config.toml");
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    skill(
        &workspace.path().join(".proteus/skills"),
        "review",
        "Project instructions.",
    );
    let plugin = workspace.path().join("package");
    skill(&plugin.join("skills"), "review", "Plugin instructions.");
    std::fs::write(plugin.join("plugin.json"),json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"test.plugin","extensions":{"org.unknown":{"hooks":"must not execute"}}}).to_string()).unwrap();
    std::fs::write(plugin.join("server.py"),r#"import json, os, sys
for line in sys.stdin:
    request=json.loads(line)
    if 'id' not in request: continue
    method=request['method']
    if method=='initialize': result={'protocolVersion':request['params']['protocolVersion'],'capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method=='tools/list': result={'tools':[{'name':'echo','description':'Fixture','inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}
    elif method=='tools/call': result={'content':[{'type':'text','text':json.dumps({'cwd':os.getcwd(),'root':os.environ['PLUGIN_ROOT'],'data':os.environ['PLUGIN_DATA'],'arg':sys.argv[1]})}],'isError':False}
    else: raise RuntimeError(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#).unwrap();
    std::fs::write(plugin.join("mcp.json"),json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{
        "echo":{"type":"stdio","command":"python3","args":["${PLUGIN_ROOT}/server.py","${PLUGIN_DATA}"]},
        "broken":{"type":"stdio","command":"missing-fixture-command"}
    }}).to_string()).unwrap();
    let marker = workspace.path().join("disabled-server-started");
    let config: AppConfig = serde_json::from_value(json!({
        "modules":{"policy":"allow_all"}, "tools":{"enabled":["skill"], "mcp_servers":[{
            "name":"disabled", "enabled":false,"command":"python3", "args":["-c",format!("open({:?},'w').write('started')",marker)]
        }]},
        "components":{"capabilities":{"command":crate::test_model::reference_module(),"exports":{"context_provider":{"skills":{}},"tool":{"reference.tools":{}},"policy":{"allow_all":{}}}}},
        "addons":{"plugins":[{"path":plugin,"enabled":true}]}
    })).unwrap();
    std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();
    let server = AgentAppServer::launch(config, workspace.path().to_path_buf(), Some(&config_path))
        .await
        .unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let state = HttpAppState::new(server.clone(), shutdown, test_security()).await;
    let unauthorized = route_request(
        state.clone(),
        Request::builder()
            .uri("/addons")
            .body(empty_body())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let response = route_request(
        state.clone(),
        authed_get_request(&session_uri("/addons", &server)),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let snapshot: AppAddonsSnapshot =
        serde_json::from_slice(&response_bytes(response).await).unwrap();
    assert!(snapshot.writable);
    let catalog = snapshot
        .catalogs
        .iter()
        .find(|catalog| catalog.provider == "skills")
        .unwrap()
        .catalog
        .as_ref()
        .unwrap();
    assert!(
        catalog
            .skills
            .iter()
            .any(|skill| skill.id == "review" && skill.enabled)
    );
    assert!(
        catalog
            .skills
            .iter()
            .any(|skill| skill.id == "test.plugin:review" && skill.enabled)
    );
    assert!(!marker.exists());
    assert!(
        snapshot
            .mcp_servers
            .iter()
            .any(|server| server.name == "test.plugin:broken" && server.error.is_some())
    );
    assert!(load_skill(&server, "review").await.ok);
    assert_eq!(
        load_skill(&server, "test.plugin:review").await.output,
        "Plugin instructions."
    );
    let echo = snapshot
        .mcp_servers
        .iter()
        .find(|server| server.name == "test.plugin:echo")
        .unwrap();
    assert!(echo.error.is_none(), "{echo:?}");
    let result = server
        .runtime
        .execute_tool(
            ToolCall::new(new_call_id(), &echo.tools[0], json!({})),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    let environment: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(environment["cwd"], plugin.to_string_lossy().as_ref());
    assert_eq!(environment["root"], plugin.to_string_lossy().as_ref());
    assert_eq!(environment["arg"], environment["data"]);
    assert!(std::path::Path::new(environment["data"].as_str().unwrap()).is_dir());

    let mut update = snapshot.settings;
    update.addons.disabled_skills = vec!["review".into()];
    update.addons.disabled_mcp_servers = vec!["test.plugin:echo".into()];
    let response = route_request(
        state.clone(),
        authed_json_request(
            &session_uri("/addons", &server),
            serde_json::to_value(&update).unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response_bytes(response).await)
    );
    assert!(!load_skill(&server, "review").await.ok);
    assert!(load_skill(&server, "test.plugin:review").await.ok);
    let disabled_result = server
        .runtime
        .execute_tool(
            ToolCall::new(new_call_id(), &echo.tools[0], json!({})),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!disabled_result.ok, "disabled MCP tool must not execute");
    let loaded = AppConfig::load(Some(&config_path)).await.unwrap();
    assert_eq!(loaded.addons, update.addons);
    let revisions = server.config_history().await.unwrap();
    assert!(
        revisions.revisions[0]
            .state
            .addon_settings
            .addons
            .disabled_skills
            .is_empty()
    );
    let saved = std::fs::read(&config_path).unwrap();
    let mut invalid: AppAddonsUpdate = update.clone();
    invalid.addons.plugins[0].path = workspace.path().join("missing-plugin");
    assert!(server.set_addons(invalid).await.is_err());
    assert_eq!(std::fs::read(&config_path).unwrap(), saved);
    update.addons.plugins[0].enabled = false;
    server.set_addons(update).await.unwrap();
    assert!(!load_skill(&server, "test.plugin:review").await.ok);
    assert!(server.transcript().await.unwrap().is_empty());
    server.shutdown().await;
}
