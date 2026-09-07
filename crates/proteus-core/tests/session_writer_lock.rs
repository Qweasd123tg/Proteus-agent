use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

use proteus_contracts::domain::{new_session_id, new_thread_id};
use proteus_contracts::model_standard::{CanonicalMessage, MessageRole};
use proteus_core::core::SessionStore;

const HELPER_ENV: &str = "PROTEUS_TEST_SESSION_WRITER_HELPER";
const CONFIG_DIR_ENV: &str = "PROTEUS_TEST_SESSION_WRITER_CONFIG_DIR";
const WORKSPACE_ENV: &str = "PROTEUS_TEST_SESSION_WRITER_WORKSPACE";
const SESSION_ID_ENV: &str = "PROTEUS_TEST_SESSION_WRITER_SESSION_ID";
const LOCKED_MARKER: &str = "PROTEUS_TEST_WRITER_LOCKED";

#[test]
#[ignore = "spawned by exclusive_session_writer_is_released_after_process_exit"]
fn session_writer_process_helper() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    let config_dir = PathBuf::from(std::env::var(CONFIG_DIR_ENV).expect("helper config dir"));
    let workspace = PathBuf::from(std::env::var(WORKSPACE_ENV).expect("helper workspace"));
    let session_id = std::env::var(SESSION_ID_ENV)
        .expect("helper session id")
        .parse()
        .expect("valid helper session id");
    let runtime = tokio::runtime::Runtime::new().expect("helper runtime");
    let store = SessionStore::new(&config_dir, &workspace, session_id).expect("helper store");
    runtime
        .block_on(store.append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::User, "first writer")],
        ))
        .expect("helper append");

    println!("{LOCKED_MARKER}");
    std::io::stdout().flush().expect("flush helper marker");
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .expect("hold helper until stdin closes");
}

#[tokio::test]
async fn exclusive_session_writer_is_released_after_process_exit() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let session_id = new_session_id();
    let store =
        SessionStore::new(config_dir.path(), workspace.path(), session_id).expect("parent store");
    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("session_writer_process_helper")
        .arg("--ignored")
        .arg("--nocapture")
        .env(HELPER_ENV, "1")
        .env(CONFIG_DIR_ENV, config_dir.path())
        .env(WORKSPACE_ENV, workspace.path())
        .env(SESSION_ID_ENV, session_id.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first writer process");
    let mut stdout = BufReader::new(child.stdout.take().expect("helper stdout"));
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = stdout.read_line(&mut line).expect("read helper output");
        assert_ne!(bytes, 0, "helper exited before acquiring writer ownership");
        if line.trim() == LOCKED_MARKER {
            break;
        }
    }

    let before = std::fs::read(store.journal_path()).expect("journal after first writer");
    let error = store
        .append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::User, "second writer")],
        )
        .await
        .expect_err("second OS process must not acquire the same write session");
    assert!(
        error
            .to_string()
            .contains("active writer in another process"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(store.journal_path()).expect("journal after rejected writer"),
        before
    );
    assert_eq!(
        store
            .load_projection()
            .expect("read-only projection")
            .history
            .len(),
        1
    );

    child.kill().expect("kill first writer process");
    child.wait().expect("reap first writer process");

    store
        .append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(
                MessageRole::User,
                "writer after crash",
            )],
        )
        .await
        .expect("OS must release writer ownership after process death");
    let projection = store.load_projection().expect("cold projection");
    assert_eq!(projection.history.len(), 2);
    assert_eq!(
        projection
            .records
            .iter()
            .map(|record| record.session_seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}
