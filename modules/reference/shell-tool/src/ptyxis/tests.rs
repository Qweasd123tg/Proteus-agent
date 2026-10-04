use super::*;
#[test]
fn ptyxis_command_title_is_single_line_and_bounded() {
    assert_eq!(command_summary("cargo test\nignored"), "cargo test");
    let title = command_summary(&"x".repeat(100));
    assert!(title.ends_with('…'));
    assert!(title.chars().count() <= 60);
}

#[test]
fn ptyxis_wrapper_streams_output_records_status_and_stays_open() {
    let wrapper = ptyxis_wrapper_script();
    assert!(wrapper.contains("tee \"$stdout_path\""));
    assert!(wrapper.contains("tee \"$stderr_path\""));
    assert!(wrapper.contains("printf '[agent] command:\\n'"));
    assert!(wrapper.contains("printf '%s\\n\\n' \"$command_text\""));
    assert!(wrapper.contains("publish_status \"$status\" \"$stdout_status\" \"$stderr_status\""));
    assert!(wrapper.contains("trap 'finish' HUP INT TERM"));
    assert!(wrapper.contains("exec bash --noprofile --norc -i"));
}

#[test]
fn ptyxis_execute_command_restores_desktop_bus_and_quotes_command() {
    let capture_dir = tempfile::tempdir().expect("capture dir");
    let paths = PtyxisCapturePaths::new(capture_dir.path());
    let execute =
        ptyxis_execute_command("printf '%s' done", &paths, Some("unix:path=/tmp/user bus"));
    assert!(execute.starts_with("env DBUS_SESSION_BUS_ADDRESS='unix:path=/tmp/user bus' bash "));
    assert!(execute.contains("'printf '\"'\"'%s'\"'\"' done'"));
}

#[cfg(unix)]
fn wrapper_fixture(command: &str, drain: &str) -> (TempDir, PtyxisCapturePaths, WrapperProcess) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let paths = PtyxisCapturePaths::new(dir.path());
    fs::write(&paths.wrapper, ptyxis_wrapper_script()).unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("tee"), drain).unwrap();
    fs::set_permissions(bin.join("tee"), fs::Permissions::from_mode(0o755)).unwrap();
    let child = Command::new("bash")
        .arg(&paths.wrapper)
        .arg(command)
        .arg(&paths.stdout)
        .arg(&paths.stderr)
        .arg(&paths.status)
        .arg(&paths.cancel)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let process = WrapperProcess {
        child,
        cancel: paths.cancel.clone(),
    };
    (dir, paths, process)
}

#[cfg(unix)]
struct WrapperProcess {
    child: std::process::Child,
    cancel: PathBuf,
}

#[cfg(unix)]
impl WrapperProcess {
    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.child.wait()
    }
}

#[cfg(unix)]
impl Drop for WrapperProcess {
    fn drop(&mut self) {
        // Assertion failures must release the same owned supervisor group.
        let _ = fs::write(&self.cancel, "");
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn wait_for_files(files: &[&Path]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while files.iter().any(|file| !file.exists()) {
        assert!(
            Instant::now() < deadline,
            "fixture never reached ready state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn completion_is_atomic_and_waits_for_both_output_drains() {
    let (dir, paths, mut child) = wrapper_fixture(
        "printf out-marker; printf err-marker >&2; exit 7",
        "#!/usr/bin/env bash\nprintf ready > \"$1.ready\"\nwhile [ ! -e \"$1.release\" ]; do /usr/bin/sleep 0.01; done\nexec /usr/bin/tee \"$@\"\n",
    );
    let stdout_ready = paths.stdout.with_extension("log.ready");
    let stderr_ready = paths.stderr.with_extension("log.ready");
    wait_for_files(&[&stdout_ready, &stderr_ready]);
    // A partially written temporary marker must remain invisible to the poller.
    fs::write(paths.status.with_extension("tmp"), "").unwrap();
    assert!(!paths.status.exists());
    fs::write(paths.stdout.with_extension("log.release"), "").unwrap();
    wait_for_files(&[&paths.stdout]);
    assert!(
        !paths.status.exists(),
        "stderr drain still owns pending output"
    );
    fs::write(paths.stderr.with_extension("log.release"), "").unwrap();
    let (output, timed_out) =
        wait_for_ptyxis_result(dir, paths, Duration::from_secs(3), &mut || Ok(false)).unwrap();
    child.wait().unwrap();
    assert!(!timed_out);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout.to_text(), "out-marker");
    assert_eq!(output.stderr.to_text(), "err-marker");
}

#[cfg(unix)]
#[test]
fn failed_drain_is_distinguished_from_command_success() {
    let (dir, paths, mut child) = wrapper_fixture(
        "printf small; exit 0",
        "#!/usr/bin/env bash\n/usr/bin/tee \"$@\"\nexit 9\n",
    );
    let error = wait_for_ptyxis_result(dir, paths, Duration::from_secs(3), &mut || Ok(false))
        .err()
        .unwrap();
    child.wait().unwrap();
    assert!(
        error.to_string().contains("output capture failed"),
        "{error}"
    );
    assert!(error.to_string().contains("command exit: 0"), "{error}");
}

#[cfg(unix)]
#[test]
fn cancellation_during_delayed_capture_stops_owned_group_without_a_pid_file() {
    let (dir, paths, mut child) = wrapper_fixture(
        "printf finished; exit 0",
        "#!/usr/bin/env bash\nprintf ready > \"$1.ready\"\n/usr/bin/sleep 1\nprintf stale-drain > \"$1.late\"\nexec /usr/bin/tee \"$@\"\n",
    );
    let stdout_ready = paths.stdout.with_extension("log.ready");
    let stderr_ready = paths.stderr.with_extension("log.ready");
    wait_for_files(&[&stdout_ready, &stderr_ready]);
    assert!(
        stop_ptyxis_command(&paths),
        "live wrapper must acknowledge group stop"
    );
    child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert!(!paths.stdout.with_extension("log.late").exists());
    assert!(!paths.stderr.with_extension("log.late").exists());
    assert!(!dir.path().join("pid").exists());
}
