//! WebKitGTK graphics configuration before GTK or other threads start.

/// Set the NVIDIA explicit-sync workaround unless the user chose a value.
///
/// # Safety
/// Call only at the beginning of `main`, before any threads or GTK startup.
pub unsafe fn configure_before_threads() {
    #[cfg(target_os = "linux")]
    if should_disable_explicit_sync(
        std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").as_deref(),
        has_nvidia_card(std::path::Path::new("/sys/class/drm")),
    ) {
        // NVIDIA 580.x can leak sync_file FDs until WebKit reaches RLIMIT_NOFILE.
        // Keep DMA-BUF accelerated rendering while bypassing that sync path:
        // https://github.com/NVIDIA/egl-wayland/issues/196
        // SAFETY: the caller guarantees there are no concurrent environment readers.
        unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
        eprintln!("Proteus: NVIDIA detected; explicit sync disabled for WebKitGTK");
    }
}

#[cfg(target_os = "linux")]
fn should_disable_explicit_sync(explicit_value: Option<&std::ffi::OsStr>, nvidia: bool) -> bool {
    nvidia && explicit_value.is_none()
}

#[cfg(target_os = "linux")]
fn has_nvidia_card(drm: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(index) = name.to_str().and_then(|name| name.strip_prefix("card")) else {
            return false;
        };
        !index.is_empty()
            && index.bytes().all(|byte| byte.is_ascii_digit())
            && std::fs::canonicalize(entry.path().join("device/driver"))
                .ok()
                .and_then(|driver| driver.file_name().map(|name| name == "nvidia"))
                .unwrap_or(false)
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{has_nvidia_card, should_disable_explicit_sync};
    use std::{ffi::OsStr, fs, os::unix::fs::symlink};

    #[test]
    fn only_a_card_bound_to_nvidia_is_detected() {
        let directory = tempfile::tempdir().unwrap();
        let drm = directory.path().join("drm");
        let nvidia = directory.path().join("drivers/nvidia");
        let nouveau = directory.path().join("drivers/nouveau");
        fs::create_dir_all(&nvidia).unwrap();
        fs::create_dir_all(&nouveau).unwrap();
        assert!(!has_nvidia_card(&drm));
        fs::create_dir_all(drm.join("card0/device")).unwrap();
        symlink(&nouveau, drm.join("card0/device/driver")).unwrap();
        fs::create_dir_all(drm.join("card0-DP-1/device")).unwrap();
        symlink(&nvidia, drm.join("card0-DP-1/device/driver")).unwrap();
        assert!(!has_nvidia_card(&drm));
        fs::create_dir_all(drm.join("card1/device")).unwrap();
        symlink(&nvidia, drm.join("card1/device/driver")).unwrap();
        assert!(has_nvidia_card(&drm));
    }

    #[test]
    fn an_explicit_value_always_wins() {
        assert!(should_disable_explicit_sync(None, true));
        assert!(!should_disable_explicit_sync(None, false));
        for value in ["", "0", "1"] {
            assert!(!should_disable_explicit_sync(Some(OsStr::new(value)), true));
        }
    }
}
