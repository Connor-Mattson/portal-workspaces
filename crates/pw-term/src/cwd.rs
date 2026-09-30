//! Finding a shell's current working directory, so restored panes reopen where you left them.

use std::path::PathBuf;

/// The current directory of the shell started as `pid`, if the platform lets us see it.
pub fn current_dir(pid: u32) -> Option<PathBuf> {
    imp::current_dir(pid)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;

    pub fn current_dir(pid: u32) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    use libproc::proc_pid::pidcwd;
    use libproc::processes::{ProcFilter, pids_by_type};

    /// On macOS the PTY's direct child is `/usr/bin/login`, which runs the shell as its child,
    /// so the interesting cwd belongs to that child when there is one.
    pub fn current_dir(pid: u32) -> Option<PathBuf> {
        let shell = pids_by_type(ProcFilter::ByParentProcess { ppid: pid })
            .ok()
            .and_then(|children| children.into_iter().min())
            .unwrap_or(pid);
        pidcwd(shell as i32).or_else(|_| pidcwd(pid as i32)).ok()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    use std::path::PathBuf;

    pub fn current_dir(_pid: u32) -> Option<PathBuf> {
        None
    }
}
