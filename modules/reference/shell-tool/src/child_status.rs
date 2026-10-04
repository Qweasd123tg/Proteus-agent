//! Observe a Unix child exit without releasing its process identity.
use std::os::unix::process::ExitStatusExt;
use std::{io, process::ExitStatus};

pub(super) fn observe_exit(pid: u32) -> io::Result<Option<ExitStatus>> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // WNOWAIT is essential: an exited but unreaped leader reserves the number
    // while cancellation/timeout may still need to stop its process group.
    loop {
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != -1 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let (pid, status) = (info.si_pid, info.si_status);
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let (pid, status) = unsafe { (info.si_pid(), info.si_status()) };
    if pid == 0 {
        return Ok(None);
    }
    let raw = if info.si_code == libc::CLD_EXITED {
        status << 8
    } else {
        status
    };
    Ok(Some(ExitStatus::from_raw(raw)))
}
