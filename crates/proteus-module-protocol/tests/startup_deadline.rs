use std::{
    fs,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, ensure};
use proteus_contracts::contracts::PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION;
use proteus_module_protocol::v3::{ComponentBroker, ComponentBrokerOptions};
use proteus_module_protocol::{ProcessComponentBinding, ProcessExportBinding};
use proteus_process_host::ProcessSpec;
use serde_json::json;

#[test]
fn initialize_write_to_nonreading_child_obeys_handshake_deadline() -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "proteus-startup-pid-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let spec = ProcessSpec::new("python3").args([
        "-c".into(),
        "import os,sys,time; open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(5)".into(),
        path.to_string_lossy().into_owned(),
    ]);
    let binding = ProcessComponentBinding::new(
        "nonreading",
        [ProcessExportBinding::new(
            "context_provider",
            "fixture.search",
            PROCESS_CONTEXT_PROVIDER_CONTRACT_VERSION,
            json!({"large_config": "x".repeat(1024 * 1024)}),
        )?],
    )?;
    let options = ComponentBrokerOptions {
        handshake_timeout: Duration::from_millis(200),
        ..ComponentBrokerOptions::default()
    };
    let started = Instant::now();
    let error = ComponentBroker::connect(spec, binding, options)
        .expect_err("initialize must time out while writing");
    ensure!(
        started.elapsed() < Duration::from_secs(2),
        "startup cleanup outlived its handshake deadline: {:?}",
        started.elapsed()
    );
    ensure!(
        format!("{error:#}").contains("timed out while writing"),
        "unexpected initialization error: {error:#}"
    );
    let pid: u32 = fs::read_to_string(&path)?.parse()?;
    fs::remove_file(path)?;
    #[cfg(target_os = "linux")]
    ensure!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "nonreading child survived startup timeout"
    );
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
    Ok(())
}
