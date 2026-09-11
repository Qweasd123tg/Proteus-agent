use super::*;

#[tokio::test]
async fn workspace_reads_are_authenticated_session_scoped_and_bounded() {
    let (state, server, root) = test_state().await;
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(
        root.path().join("src/hello world.txt"),
        "<script>Привет</script>\n",
    )
    .unwrap();
    std::fs::write(root.path().join("binary"), [0, 255]).unwrap();
    std::fs::write(root.path().join("large"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    let unauthorized = route_request(
        state.clone(),
        Request::builder()
            .uri("/workspace/list")
            .body(empty_body())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    for (suffix, expected) in [
        ("/workspace/list", StatusCode::BAD_REQUEST),
        (
            "/workspace/file?path=src%2Fhello%20world.txt",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = route_request(state.clone(), authed_get_request(suffix))
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "session must be explicit");
    }
    for (endpoint, path, kind, text) in [
        (
            "/workspace/file",
            "src%2Fhello%20world.txt",
            "text",
            Some("<script>Привет</script>\n"),
        ),
        ("/workspace/file", "binary", "binary", None),
        ("/workspace/file", "large", "too_large", None),
    ] {
        let uri = format!("{}&path={path}", session_uri(endpoint, &server));
        let response = route_request(state.clone(), authed_get_request(&uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&response_bytes(response).await).unwrap();
        assert_eq!(body["kind"], kind);
        assert_eq!(body["text"].as_str(), text);
    }
    let response = route_request(
        state.clone(),
        authed_get_request(&session_uri("/workspace/list", &server)),
    )
    .await
    .unwrap();
    let body: Value = serde_json::from_slice(&response_bytes(response).await).unwrap();
    assert!(
        body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "src" && entry["kind"] == "directory")
    );
    for path in [
        "../secret",
        "%2Fetc%2Fpasswd",
        "src&path=binary",
        "src&unknown=1",
    ] {
        let uri = format!("{}&path={path}", session_uri("/workspace/file", &server));
        assert_eq!(
            route_request(state.clone(), authed_get_request(&uri))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let uri = format!(
            "{}&path=link%2Fsecret",
            session_uri("/workspace/file", &server)
        );
        assert_eq!(
            route_request(state.clone(), authed_get_request(&uri))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    server.shutdown().await;
}
