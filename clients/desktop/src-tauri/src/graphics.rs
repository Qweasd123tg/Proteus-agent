//! WebKitGTK startup configuration, before GTK or the async runtime starts.

/// Apply the verified NVIDIA DMA-BUF workaround, preserving explicit overrides.
///
/// # Safety
/// Call only at the beginning of main, before any threads or GTK initialization.
pub unsafe fn configure_before_threads() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        && has_nvidia_card(std::path::Path::new("/sys/class/drm"))
    {
        // NVIDIA's EGL path can crash both WebKit and GTK while a second window
        // opens/resizes. See https://v2.tauri.app/develop/debug/linux-graphics/.
        // SAFETY: the caller guarantees that no other thread can read the env.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        eprintln!("Proteus: NVIDIA detected; WebKit DMA-BUF renderer disabled");
    }
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
    use super::has_nvidia_card;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn only_a_card_bound_to_nvidia_enables_the_workaround() {
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
}
