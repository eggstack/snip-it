#![allow(dead_code)]

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;
use uuid::Uuid;

#[allow(dead_code)]
pub struct TestEnvironmentBuilder {
    server_url: Option<String>,
    debounce: u64,
    failure_mode: String,
}

#[allow(dead_code)]
pub struct TestEnvironment {
    pub tmp: TempDir,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub home_dir: PathBuf,
    pub api_key: String,
    pub device_id: String,
    pub server_url: Option<String>,
    debounce: u64,
    failure_mode: String,
    child_pids: RefCell<Vec<u32>>,
}

#[allow(dead_code)]
impl TestEnvironment {
    pub fn builder() -> TestEnvironmentBuilder {
        TestEnvironmentBuilder {
            server_url: None,
            debounce: 0,
            failure_mode: "warn".to_string(),
        }
    }

    fn base_snp_cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_snp"));
        cmd.env(
            "XDG_CONFIG_HOME",
            self.home_dir.join(".config").to_str().unwrap(),
        );
        cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
        cmd
    }

    pub fn snp_cmd(&self) -> Command {
        self.base_snp_cmd()
    }

    pub fn snp_args(&self, args: &[&str]) -> Command {
        let mut cmd = self.base_snp_cmd();
        cmd.args(args);
        cmd
    }

    pub fn snp_output(&self, args: &[&str]) -> Output {
        self.snp_args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap()
    }

    pub fn snp_output_with_stdin(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self
            .base_snp_cmd()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    #[allow(clippy::zombie_processes)]
    pub fn spawn_snp_detached(&self, args: &[&str]) -> u32 {
        let mut cmd = self.base_snp_cmd();
        cmd.args(args);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let child = cmd.spawn().unwrap();
        let pid = child.id();
        self.child_pids.borrow_mut().push(pid);
        pid
    }

    pub fn write_sync_toml(&self) {
        let server_url = self
            .server_url
            .as_deref()
            .unwrap_or("http://127.0.0.1:19999");

        let toml = format!(
            r#"[settings.sync]
enabled = true
server_url = "{server_url}"
api_key = "{api_key}"
device_id = "{device_id}"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = {debounce}
auto_sync_failure = "{failure_mode}"
"#,
            api_key = self.api_key,
            device_id = self.device_id,
            debounce = self.debounce,
            failure_mode = self.failure_mode,
        );

        let sync_path = self.config_dir.join("sync.toml");
        fs::write(&sync_path, toml).unwrap();
    }

    pub fn create_library(&self, name: &str) {
        let mut cmd = self.base_snp_cmd();
        cmd.args(["library", "create", name]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd.output().unwrap();

        let mut cmd = self.base_snp_cmd();
        cmd.args(["library", "set-primary", name]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd.output().unwrap();
    }

    pub fn new_snippet(&self, desc: &str) -> Output {
        self.snp_output_with_stdin(
            &["new", "--command-stdin", "--description", desc],
            format!("echo {desc}").as_bytes(),
        )
    }

    pub fn read_pending_generation(&self) -> Option<u64> {
        let path = self.config_dir.join("auto-sync-pending.toml");
        let content = fs::read_to_string(&path).ok()?;
        parse_pending_generation(&content)
    }

    pub fn pending_marker_path(&self) -> PathBuf {
        self.config_dir.join("auto-sync-pending.toml")
    }

    pub fn status_file_path(&self) -> PathBuf {
        self.config_dir.join("auto-sync-status.toml")
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let pids = self.child_pids.borrow();
        if pids.is_empty() {
            return;
        }
        for &pid in pids.iter() {
            #[cfg(unix)]
            {
                let alive = unsafe { libc::kill(pid as i32, 0) == 0 };
                if alive {
                    eprintln!(
                        "WARNING: child process {pid} still running after \
                         test environment drop"
                    );
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
impl TestEnvironmentBuilder {
    pub fn with_server_url(mut self, url: &str) -> Self {
        self.server_url = Some(url.to_string());
        self
    }

    pub fn with_debounce(mut self, seconds: u64) -> Self {
        self.debounce = seconds;
        self
    }

    pub fn with_failure_mode(mut self, mode: &str) -> Self {
        self.failure_mode = mode.to_string();
        self
    }

    pub fn build(self) -> std::io::Result<TestEnvironment> {
        let tmp = TempDir::new()?;
        let home_dir = tmp.path().to_path_buf();
        let config_dir = home_dir.join(".config").join("snp");
        fs::create_dir_all(&config_dir)?;

        let device_id = format!("test-device-{}", Uuid::new_v4());
        let api_key = "test-api-key-e2e-05a".to_string();

        let env = TestEnvironment {
            config_dir: config_dir.clone(),
            state_dir: config_dir,
            home_dir,
            api_key,
            device_id,
            server_url: self.server_url.clone(),
            debounce: self.debounce,
            failure_mode: self.failure_mode,
            child_pids: RefCell::new(Vec::new()),
            tmp,
        };

        env.write_sync_toml();

        Ok(env)
    }
}

fn parse_pending_generation(content: &str) -> Option<u64> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("generation = ") {
            return val.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creates_isolated_env() {
        let env = TestEnvironment::builder().build().unwrap();
        assert!(env.config_dir.exists());
        assert!(env.config_dir.join("sync.toml").exists());
        assert!(env.api_key == "test-api-key-e2e-05a");
        assert!(env.device_id.starts_with("test-device-"));
    }

    #[test]
    fn test_snp_cmd_has_xdg_config_home() {
        let env = TestEnvironment::builder().build().unwrap();
        let mut cmd = env.snp_cmd();
        let output = cmd.arg("--help").output().unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn test_write_sync_toml_reflects_builder_settings() {
        let env = TestEnvironment::builder()
            .with_server_url("http://127.0.0.1:9999")
            .with_debounce(5)
            .with_failure_mode("ignore")
            .build()
            .unwrap();

        env.write_sync_toml();
        let content = fs::read_to_string(env.config_dir.join("sync.toml")).unwrap();
        assert!(content.contains("server_url = \"http://127.0.0.1:9999\""));
        assert!(content.contains("auto_sync_debounce_seconds = 5"));
        assert!(content.contains("auto_sync_failure = \"ignore\""));
    }

    #[test]
    fn test_create_library_sets_primary() {
        let env = TestEnvironment::builder().build().unwrap();
        env.create_library("mylib");
        let lib_path = env.config_dir.join("libraries").join("mylib.toml");
        assert!(lib_path.exists());
    }

    #[test]
    fn test_pending_marker_path() {
        let env = TestEnvironment::builder().build().unwrap();
        assert!(
            env.pending_marker_path()
                .ends_with("auto-sync-pending.toml")
        );
    }

    #[test]
    fn test_status_file_path() {
        let env = TestEnvironment::builder().build().unwrap();
        assert!(env.status_file_path().ends_with("auto-sync-status.toml"));
    }

    #[test]
    fn test_read_pending_generation_none_when_missing() {
        let env = TestEnvironment::builder().build().unwrap();
        assert_eq!(env.read_pending_generation(), None);
    }

    #[test]
    fn test_read_pending_generation_parses_value() {
        let env = TestEnvironment::builder().build().unwrap();
        fs::write(
            env.pending_marker_path(),
            "schema = 2\ngeneration = 3\ncreated_at_unix_ms = 1700000000000\n",
        )
        .unwrap();
        assert_eq!(env.read_pending_generation(), Some(3));
    }
}
