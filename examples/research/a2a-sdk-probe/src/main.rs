//! Candidate gate: real SDK client/server over loopback HTTP, synthetic executor.
//! Exit 1 means a required behavior is absent; it is not an expected-output test
//! that blesses the pinned SDK's defects as the desired contract.

mod fixture;
mod scenarios;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match scenarios::run().await {
        Ok(checks) => {
            let passed = checks
                .iter()
                .filter(|check| check["passed"] == true)
                .count();
            let report = serde_json::json!({
                "sdk_repository": "https://github.com/a2aproject/a2a-rs",
                "sdk_revision": "c7cefa0b4276805efbcfd2ddd3238c22e5f36b8f",
                "sdk_packages": {"a2a-lf": "0.3.0", "a2a-client-lf": "0.2.3", "a2a-server-lf": "0.4.3"},
                "transport": "JSON-RPC over loopback HTTP; SDK SSE for subscription check",
                "executor": "synthetic deterministic fixture, not a Proteus runtime",
                "passed": passed,
                "total": checks.len(),
                "checks": checks,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("serialize report")
            );
            if passed == checks.len() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("A2A probe could not complete: {error:#}");
            ExitCode::from(2)
        }
    }
}
