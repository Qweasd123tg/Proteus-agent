use super::*;
use proteus_contracts::{
    contracts::ProcessModelDescriptor,
    model_standard::{
        CanonicalMessage, CanonicalModelResponse, FinishReason, MessageRole, ModelCapabilities,
    },
};
use serde_json::Value;

fn option<'a>(options: &'a Value, id: &str) -> &'a Value {
    options
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap()
}

async fn change(client: &mut Client, id: u64, session: &str, key: &str, value: &str) -> Value {
    client
        .request(
            id,
            "session/set_config_option",
            json!({"sessionId":session,"configId":key,"value":value}),
        )
        .await;
    let (response, updates) = client.response(id).await;
    assert!(response.get("error").is_none(), "{response}");
    let options = response["result"]["configOptions"].clone();
    assert!(
        updates
            .iter()
            .any(|u| u["params"]["update"]["configOptions"] == options)
    );
    options
}

#[tokio::test]
async fn selectors_use_external_catalog_and_apply_to_journaled_request() {
    let mut client = Client::launch_config(1, 300_000, |config| {
        let response = CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "selected model answered"),
            vec![], FinishReason::Stop,
        );
        config.components.remove("test-model");
        config.components.insert("independent".into(), serde_json::from_value(json!({
            "command":"python3", "args":["-B", std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/process_model.py")],
            "exports":{"model":{"external":{}}}
        })).unwrap());
        config.providers.clear();
        config.active_provider = "external".into();
        config.providers.insert("external".into(), serde_json::from_value(json!({
            "provider":"external","model":"first","stream":true,
            "reasoning":{"effort":"high","summary":true}
        })).unwrap());
        config.module_config.entry("model".into()).or_default().insert("external".into(), json!({
            "descriptor":ProcessModelDescriptor { adapter_id:"external".into(),
                capabilities:ModelCapabilities::basic_text_and_tools().with_streaming(true).with_reasoning_config(true),
                hosted_tools:vec![] },
            "catalog":{"models":[
                {"id":"first","display_name":"First","hidden":false,"description":null,
                    "reasoning_efforts":["high","ultra"],"default_reasoning_effort":"high"},
                {"id":"second","display_name":"Second","hidden":false,"description":null,
                    "reasoning_efforts":["none","low"],"default_reasoning_effort":"low"},
                {"id":"hidden","display_name":"Hidden","hidden":true,"description":null,
                    "reasoning_efforts":[],"default_reasoning_effort":null}
            ]},
            "terminal":{"kind":"response","response":response}
        }));
    }).await;
    client.initialize().await;
    client
        .request(2, "session/new", json!({"cwd":client.cwd,"mcpServers":[]}))
        .await;
    let response = client.response(2).await.0;
    let session = response["result"]["sessionId"]
        .as_str()
        .expect(&response.to_string())
        .to_owned();
    let initial = &response["result"]["configOptions"];
    assert_eq!(option(initial, "model")["currentValue"], "first");
    assert_eq!(
        option(initial, "model")["options"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        option(initial, "reasoning_effort")["currentValue"],
        "effort:high"
    );
    let second_session = client.new_session(3).await;
    let options = change(&mut client, 4, &session, "model", "second").await;
    assert_eq!(option(&options, "model")["currentValue"], "second");
    assert_eq!(
        option(&options, "reasoning_effort")["currentValue"],
        "effort:low"
    );
    assert_eq!(
        option(&options, "reasoning_effort")["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["_default", "effort:none", "effort:low"]
    );
    for (id, key, value) in [
        (5, "model", "hidden"),
        (6, "reasoning_effort", "effort:ultra"),
        (7, "missing", "value"),
    ] {
        client
            .request(
                id,
                "session/set_config_option",
                json!({"sessionId":session,"configId":key,"value":value}),
            )
            .await;
        assert_eq!(client.response(id).await.0["error"]["code"], -32602);
    }
    client
        .request(
            8,
            "session/set_config_option",
            json!({"sessionId":session,"configId":"mode","type":"boolean","value":true}),
        )
        .await;
    assert_eq!(client.response(8).await.0["error"]["code"], -32602);
    let options = change(&mut client, 9, &session, "reasoning_effort", "_default").await;
    assert_eq!(
        option(&options, "reasoning_effort")["currentValue"],
        "_default"
    );
    change(&mut client, 10, &session, "reasoning_effort", "effort:none").await;
    let options = change(&mut client, 11, &session, "mode", "plan").await;
    assert_eq!(option(&options, "mode")["currentValue"], "plan");
    client
        .request(
            12,
            "session/set_mode",
            json!({"sessionId":session,"modeId":"normal"}),
        )
        .await;
    let (_, updates) = client.response(12).await;
    let config_update = updates
        .iter()
        .find(|u| u["params"]["update"]["sessionUpdate"] == "config_option_update")
        .unwrap();
    assert_eq!(
        option(&config_update["params"]["update"]["configOptions"], "mode")["currentValue"],
        "normal"
    );
    for (id, session) in [(13, &session), (14, &second_session)] {
        client.prompt(id, session, "hello").await;
        assert_eq!(
            client.response(id).await.0["result"]["stopReason"],
            "end_turn"
        );
    }
    client.close().await;
    for (session, model, effort) in [
        (&session, "second", "none"),
        (&second_session, "first", "high"),
    ] {
        let store = client.store(session);
        let records = store.load_records().unwrap();
        let request = records
            .iter()
            .find_map(|r| match &r.entry {
                JournalEntry::ModelRequestRecorded(r) => Some(&r.request),
                _ => None,
            })
            .unwrap();
        assert_eq!(request.model.model, model);
        assert_eq!(request.reasoning.effort.as_deref(), Some(effort));
        if effort == "none" {
            assert!(!request.reasoning.summary);
        }
        let replay = replay_workflow(
            store.journal_path(),
            &client.config,
            &ModuleCatalog::from_config(&client.config).unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        assert!(replay.comparison.matched, "{:?}", replay.comparison);
    }
}

#[tokio::test]
async fn selectors_reject_changes_while_prompt_awaits_permission() {
    let mut client = Client::launch(1).await;
    client.initialize().await;
    let session = client.new_session(2).await;
    client.prompt(3, &session, "apply_patch").await;
    loop {
        if client.read().await["method"] == "session/request_permission" {
            break;
        }
    }
    client
        .request(
            4,
            "session/set_config_option",
            json!({"sessionId":session,"configId":"mode","value":"auto"}),
        )
        .await;
    assert_eq!(client.response(4).await.0["error"]["code"], -32602);
    client.cancel(&session).await;
    assert_eq!(
        client.response(3).await.0["result"]["stopReason"],
        "cancelled"
    );
    let options = change(&mut client, 5, &session, "mode", "normal").await;
    assert_eq!(option(&options, "mode")["currentValue"], "normal");
    client.close().await;
    assert!(!client.cwd.join("smoke.txt").exists());
}

#[tokio::test]
async fn configured_choices_keep_the_default_after_reasoning_is_disabled() {
    let mut client = Client::launch_config(1, 300_000, |config| {
        config.providers.get_mut("fake").unwrap().reasoning.effort = Some("high".into());
        let mut second = config.providers["fake"].clone();
        second.model = "another-model".into();
        config.providers.insert("second".into(), second);
    })
    .await;
    client.initialize().await;
    let session = client.new_session(2).await;
    let options = change(&mut client, 3, &session, "reasoning_effort", "effort:none").await;
    assert!(
        option(&options, "reasoning_effort")["options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|option| option["value"] == "effort:high")
    );
    change(&mut client, 4, &session, "reasoning_effort", "effort:high").await;
    let options = change(&mut client, 5, &session, "model", "another-model").await;
    assert_eq!(option(&options, "model")["currentValue"], "another-model");
    client.close().await;
}
