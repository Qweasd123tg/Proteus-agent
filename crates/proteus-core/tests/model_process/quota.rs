use super::*;

fn quota() -> Value {
    json!({"observed_at":1900000000,"plan":"external-plan","credits":null,"buckets":[{
        "id":"custom-resource","name":"Custom resource","allowed":true,"limit_reached":false,
        "windows":[{"id":"rolling","used_percent":42.5,"duration_seconds":90,"resets_at":1900000042}]
    }]})
}

#[tokio::test]
async fn external_quota_crosses_snapshot_boundary_without_inference() {
    use proteus_core::app_server::AgentAppServer;
    let cwd = tempfile::tempdir().unwrap();
    for id in ["python_a", "unrelated_b"] {
        let mut settings = settings();
        settings["quota"] = quota();
        // No stream result exists: quota must not attempt an inference call.
        settings.as_object_mut().unwrap().remove("terminal");
        let server = AgentAppServer::launch(config(id, settings), cwd.path().to_owned(), None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(server.model_quota().await.unwrap().unwrap()).unwrap(),
            quota()
        );
        server.shutdown().await;
    }
    let adapter = model(&config("unsupported", settings()), cwd.path()).unwrap();
    assert!(adapter.quota().await.unwrap().is_none());
}

#[tokio::test]
async fn external_quota_rejects_invalid_shapes_and_values() {
    let cwd = tempfile::tempdir().unwrap();
    let mut unknown = quota();
    unknown["buckets"][0]["windows"][0]["raw_provider_field"] = json!(1);
    let mut duplicate = quota();
    duplicate["buckets"]
        .as_array_mut()
        .unwrap()
        .push(quota()["buckets"][0].clone());
    let mut negative = quota();
    negative["buckets"][0]["windows"][0]["used_percent"] = json!(-1);
    let mut zero = quota();
    zero["buckets"][0]["windows"][0]["duration_seconds"] = json!(0);
    for invalid in [unknown, duplicate, negative, zero] {
        let mut settings = settings();
        settings["quota"] = invalid;
        let adapter = model(&config("external", settings), cwd.path()).unwrap();
        assert!(adapter.quota().await.is_err());
    }
}

#[tokio::test]
async fn dropping_quota_lookup_cancels_the_process_invocation() {
    let cwd = tempfile::tempdir().unwrap();
    let marker = cwd.path().join("quota-cancel");
    let mut settings = settings();
    settings["quota_wait"] = json!(true);
    settings["quota_marker"] = json!(marker);
    let adapter = model(&config("external", settings), cwd.path()).unwrap();
    let reader = adapter.clone();
    let lookup = tokio::spawn(async move { reader.quota().await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(&marker).ok().as_deref() != Some("started") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    lookup.abort();
    let _ = lookup.await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(&marker).ok().as_deref() != Some("canceled") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
