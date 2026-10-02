//! Keep native IPC and the packaged backend outside an inherited HTTP proxy.
use std::ffi::{OsStr, OsString};

const LOCAL_ADDRESSES: &str = "localhost,127.0.0.1,::1,tauri.localhost,ipc.localhost";

/// # Safety
/// Call before starting GTK or any application threads.
pub unsafe fn configure_before_threads() {
    let upper = std::env::var_os("NO_PROXY");
    let lower = std::env::var_os("no_proxy");
    let upper_value = with_local_addresses(upper.as_deref().or(lower.as_deref()));
    let lower_value = with_local_addresses(lower.as_deref().or(upper.as_deref()));
    // SAFETY: the caller guarantees exclusive access to the process environment.
    unsafe {
        std::env::set_var("NO_PROXY", upper_value);
        std::env::set_var("no_proxy", lower_value);
    }
}

fn with_local_addresses(existing: Option<&OsStr>) -> OsString {
    let mut value = existing.unwrap_or_default().to_os_string();
    if !value.is_empty() {
        value.push(",");
    }
    value.push(LOCAL_ADDRESSES);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_routes_preserve_existing_proxy_bypass() {
        for existing in [None, Some(OsStr::new(""))] {
            assert_eq!(
                with_local_addresses(existing),
                OsString::from(LOCAL_ADDRESSES)
            );
        }
        assert_eq!(
            with_local_addresses(Some(OsStr::new(".example.org,10.0.0.0/8"))),
            OsString::from(format!(".example.org,10.0.0.0/8,{LOCAL_ADDRESSES}"))
        );
    }
}
