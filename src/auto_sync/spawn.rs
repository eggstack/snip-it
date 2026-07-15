//! Platform-detached process spawning for the one-shot worker.

use std::path::Path;
use std::process::{Command, Stdio};

pub const WORKER_SUBCOMMAND: &str = "auto-sync-worker";

#[derive(Debug)]
pub enum SpawnError {
    Spawn(std::io::Error),
    NoExecutable,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
            Self::NoExecutable => write!(f, "could not locate snp executable"),
        }
    }
}

impl std::error::Error for SpawnError {}

pub fn spawn_worker(state_dir: &Path, nonce: &str) -> Result<u32, SpawnError> {
    let exe = std::env::current_exe().map_err(SpawnError::Spawn)?;
    let exe_path = exe.to_string_lossy().to_string();

    let mut cmd = Command::new(&exe_path);
    cmd.arg(WORKER_SUBCOMMAND);
    cmd.arg("--state-dir");
    cmd.arg(state_dir.as_os_str());
    cmd.arg("--nonce");
    cmd.arg(nonce);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    apply_platform_detach(&mut cmd);

    let child = cmd.spawn().map_err(SpawnError::Spawn)?;
    Ok(child.id())
}

#[cfg(unix)]
fn apply_platform_detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_platform_detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_subcommand_name() {
        assert_eq!(WORKER_SUBCOMMAND, "auto-sync-worker");
    }

    #[test]
    fn test_spawn_error_display() {
        let e = SpawnError::NoExecutable;
        assert_eq!(e.to_string(), "could not locate snp executable");
    }

    #[test]
    fn test_spawn_error_io_display() {
        let io_err = std::io::Error::other("boom");
        let e = SpawnError::Spawn(io_err);
        assert!(e.to_string().contains("spawn failed"));
        assert!(e.to_string().contains("boom"));
    }
}
