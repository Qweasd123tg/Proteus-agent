//! Pinned Codex terminal branches exercised with real local processes.
use super::*;

#[test]
fn pipes_are_default_and_stdin_is_eof_while_tty_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    for tty in [false, true] {
        let mut args = json!({"cmd": "if [ -t 0 ] && [ -t 1 ] && [ -t 2 ]; then printf tty; else cat; printf pipes-eof; fi"});
        if tty {
            args["tty"] = json!(true);
        }
        let result = exec_command(root.path(), args);
        assert_eq!(result["metadata"]["tty"], tty);
        assert_eq!(result["metadata"]["exit_code"], 0);
        assert!(result["output"].as_str().unwrap().ends_with(if tty {
            "tty"
        } else {
            "pipes-eof"
        }));
    }
}

#[cfg(unix)]
#[test]
fn pipe_stdin_rejects_input_but_ctrl_c_interrupts_the_command() {
    let root = tempfile::tempdir().unwrap();
    let context = invocation_context(root.path());
    let result =
        exec_command_with_context(&context, json!({"cmd": "sleep 20", "yield_time_ms": 250}));
    let id = result["metadata"]["session_id"].as_i64().unwrap();
    for chars in ["hello\n", "\u{4}", "\u{3}extra"] {
        let error =
            write_stdin_result(&context, json!({"session_id": id, "chars": chars})).unwrap_err();
        assert_eq!(
            error.to_string(),
            "stdin is closed for this session; rerun exec_command with tty=true to keep stdin open"
        );
        assert!(lock(sessions()).contains_key(&id));
    }
    let result = write_stdin(
        &context,
        json!({"session_id": id, "chars": "\u{3}", "yield_time_ms": 5000}),
    );
    assert_eq!(result["ok"], true);
    assert_eq!(result["metadata"]["exit_code"], 130, "{result}");
    assert_eq!(result["metadata"]["session_id"], Value::Null);
    assert!(!lock(sessions()).contains_key(&id));
}

#[test]
fn long_empty_poll_returns_final_stdout_and_stderr_once() {
    let root = tempfile::tempdir().unwrap();
    let context = invocation_context(root.path());
    let first = exec_command_with_context(
        &context,
        json!({"cmd": "printf first; sleep 0.8; printf last-out; printf last-err >&2; exit 7", "yield_time_ms": 250}),
    );
    assert!(first["output"].as_str().unwrap().contains("first"));
    let id = first["metadata"]["session_id"].as_i64().unwrap();
    let started = Instant::now();
    let last = write_stdin(&context, json!({"session_id": id, "yield_time_ms": 300000}));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(last["metadata"]["yield_time_ms"], 300000);
    assert_eq!(last["metadata"]["exit_code"], 7);
    assert_eq!(last["ok"], true);
    let text = last["output"].as_str().unwrap();
    assert!(!text.contains("first"));
    assert!(
        text.contains("last-out") && text.contains("last-err"),
        "{text}"
    );
    assert!(write_stdin_result(&context, json!({"session_id": id})).is_err());
}

#[test]
fn bounded_buffer_keeps_the_original_head_and_latest_tail() {
    let root = tempfile::tempdir().unwrap();
    let result = exec_command(
        root.path(),
        json!({
            "cmd": r"printf original-head; head -c 1200000 /dev/zero | tr '\000' x; printf latest-tail",
            "max_output_tokens": 350000
        }),
    );
    assert_eq!(result["metadata"]["exit_code"], 0);
    assert_eq!(result["metadata"]["output_bytes"], SESSION_BUFFER_LIMIT);
    assert_eq!(
        result["metadata"]["dropped_bytes"],
        1200024 - SESSION_BUFFER_LIMIT
    );
    let output = result["output"].as_str().unwrap();
    assert!(output.contains("original-head"));
    assert!(output.ends_with("latest-tail"));
    assert!(output.contains("omitted"));
    assert_eq!(result["metadata"]["truncated"], true);
    assert!(
        output.len() > 100000,
        "explicit token budget must not silently cap at 25000"
    );
    let zero = exec_command(
        root.path(),
        json!({"cmd": "printf hidden", "max_output_tokens": 0}),
    );
    assert!(!zero["output"].as_str().unwrap().contains("hidden"));
}

#[test]
fn poll_and_input_have_different_timeout_bounds() {
    assert_eq!(resolve_write_yield_time_ms(None, ""), 5000);
    assert_eq!(resolve_write_yield_time_ms(None, "input"), 250);
    let args = json!({"yield_time_ms": u64::MAX});
    assert_eq!(resolve_write_yield_time_ms(Some(&args), ""), 300000);
    assert_eq!(resolve_write_yield_time_ms(Some(&args), "input"), 30000);
    assert_eq!(
        resolve_yield_time_ms(Some(&args), DEFAULT_EXEC_YIELD_MS),
        30000
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_poll_kills_pipe_process_group_and_removes_session() {
    let root = tempfile::tempdir().unwrap();
    let context = invocation_context(root.path());
    let first = exec_command_with_context(
        &context,
        json!({
            "cmd": "sleep 20 & echo $! > child.pid; wait", "yield_time_ms": 250
        }),
    );
    let id = first["metadata"]["session_id"].as_i64().unwrap();
    let child = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let trigger = cancelled.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        trigger.store(true, AtomicOrdering::SeqCst);
    });
    let call = json!({"id": "cancel_poll", "name": "write_stdin", "args": {"session_id": id, "yield_time_ms": 300000}});
    let error = with_host(cancelled, |host| {
        write_stdin_impl(
            &call.to_string(),
            &serde_json::to_string(&context).unwrap(),
            host,
        )
    })
    .unwrap_err();
    canceller.join().unwrap();
    assert!(error.to_string().contains("canceled"));
    assert!(!lock(sessions()).contains_key(&id));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.trim()));
        let running =
            stat.is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with(['Z', 'X']));
        if !running {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "descendant survived cancellation"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
