#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};

const DEAD_PID: u32 = 99_999_999;

fn snip_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_snip-sync")
}

fn isolated_command(root: &Path) -> Command {
    let mut command = Command::new(snip_sync_bin());
    command.env("XDG_STATE_HOME", root);
    command.env("HOME", root);
    command
}

fn state_dir(root: &Path) -> PathBuf {
    root.join("snip-sync")
}

fn pid_path(root: &Path) -> PathBuf {
    state_dir(root).join("snip-sync.pid")
}

fn output_text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn stop_cleans_dead_legacy_pid_file() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(state_dir(root.path())).unwrap();
    fs::write(pid_path(root.path()), format!("{DEAD_PID}\n")).unwrap();

    let output = isolated_command(root.path())
        .args(["stop"])
        .output()
        .unwrap();
    let (stdout, stderr) = output_text(&output);

    assert!(
        output.status.success(),
        "status={:?}\nstdout={stdout}\nstderr={stderr}",
        output.status.code()
    );
    assert!(
        !pid_path(root.path()).exists(),
        "stale PID file remains; stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Cleaning up stale PID file"),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        !stderr.contains("Refusing to stop non-snip-sync process"),
        "stdout={stdout}\nstderr={stderr}"
    );
}

#[test]
fn restart_refuses_live_unrelated_legacy_pid_and_preserves_file() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(state_dir(root.path())).unwrap();

    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let mut child_guard = ChildGuard(Some(child));
    let child_pid = child_guard.0.as_ref().unwrap().id();
    let original_pid = format!("{child_pid}\n");
    fs::write(pid_path(root.path()), &original_pid).unwrap();

    let output = isolated_command(root.path())
        .args(["restart"])
        .output()
        .unwrap();
    let (stdout, stderr) = output_text(&output);

    assert!(
        !output.status.success(),
        "status={:?}\nstdout={stdout}\nstderr={stderr}",
        output.status.code()
    );
    assert!(
        stderr.contains("does not appear to be a snip-sync process"),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        !stdout.contains("Starting server..."),
        "stdout={stdout}\nstderr={stderr}"
    );
    assert!(
        pid_path(root.path()).exists(),
        "PID file was removed; stdout={stdout}\nstderr={stderr}"
    );
    assert_eq!(
        fs::read_to_string(pid_path(root.path())).unwrap(),
        original_pid,
        "PID file changed; stdout={stdout}\nstderr={stderr}"
    );
    assert_eq!(
        child_guard.0.as_mut().unwrap().try_wait().unwrap(),
        None,
        "unrelated process exited; stdout={stdout}\nstderr={stderr}"
    );
}
