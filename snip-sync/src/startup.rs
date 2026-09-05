//! Cross-platform startup registration for the foreground `snip-sync` server.
//!
//! This module renders small supervisor records and owns only the records it
//! writes. The server remains a normal foreground process; cron and Task
//! Scheduler invoke `croncheck`, while systemd and launchd invoke `serve`.

use crate::cli::StartupMethodArg;
use crate::{Config, parse_bool_env};
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub const SYSTEMD_UNIT_NAME: &str = "snip-sync.service";
pub const LAUNCHD_LABEL: &str = "com.eggstack.snip-sync";
pub const OWNERSHIP_MARKER: &str = "snip-sync startup managed file";
pub const CRON_BEGIN: &str = "# snip-sync managed startup (begin)";
pub const CRON_END: &str = "# snip-sync managed startup (end)";
pub const TASK_STARTUP_NAME: &str = "snip-sync startup (startup)";
pub const TASK_WATCHDOG_NAME: &str = "snip-sync startup (watchdog)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupMethod {
    Systemd,
    Launchd,
    Cron,
    TaskScheduler,
    Direct,
}

/// Whether an update must activate a new server process after replacing the
/// executable. The installed manager is retained so restart uses the same
/// lifecycle path that owns the service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateLifecycle {
    ManagedRunning(StartupMethod),
    ManagedStopped(StartupMethod),
    DirectRunning,
    NotRunning,
}

impl StartupMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::Launchd => "launchd",
            Self::Cron => "cron",
            Self::TaskScheduler => "task-scheduler",
            Self::Direct => "direct",
        }
    }
}

impl std::str::FromStr for StartupMethod {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "systemd" => Ok(Self::Systemd),
            "launchd" => Ok(Self::Launchd),
            "cron" => Ok(Self::Cron),
            "task-scheduler" => Ok(Self::TaskScheduler),
            "direct" => Ok(Self::Direct),
            other => Err(format!("unknown startup method {other:?}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportEnvironment {
    pub entries: Vec<(String, String)>,
}

pub fn method_from_arg(method: StartupMethodArg) -> Option<StartupMethod> {
    Some(match method {
        StartupMethodArg::Auto => return None,
        StartupMethodArg::Systemd => StartupMethod::Systemd,
        StartupMethodArg::Launchd => StartupMethod::Launchd,
        StartupMethodArg::Cron => StartupMethod::Cron,
        StartupMethodArg::TaskScheduler => StartupMethod::TaskScheduler,
    })
}

pub fn resolve_method(method: StartupMethodArg) -> Result<StartupMethod, String> {
    if let Some(method) = method_from_arg(method) {
        return Ok(method);
    }
    auto_detect_method()
}

pub fn auto_detect_method() -> Result<StartupMethod, String> {
    #[cfg(target_os = "linux")]
    {
        if systemd_is_running() {
            return Ok(StartupMethod::Systemd);
        }
        if command_available("crontab") {
            return Ok(StartupMethod::Cron);
        }
        Err(
            "systemd is not running and no POSIX crontab command is available; use startup instructions --method systemd or install cron"
                .into(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(StartupMethod::Launchd);
    }
    #[cfg(windows)]
    {
        return Ok(StartupMethod::TaskScheduler);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        if command_available("crontab") {
            Ok(StartupMethod::Cron)
        } else {
            Err("no supported startup manager or POSIX crontab command is available".into())
        }
    }
}

/// A bounded systemd check. Merely finding `systemctl` is intentionally not
/// enough: containers commonly have the client binary but no systemd PID 1.
pub fn systemd_is_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        if Path::new("/run/systemd/system").exists() {
            return true;
        }
        if fs::read_to_string("/proc/1/comm")
            .ok()
            .is_some_and(|value| value.trim() == "systemd")
        {
            return true;
        }
        if !command_available("systemctl") || !command_available("timeout") {
            return false;
        }
        let output = Command::new("timeout")
            .args(["--signal=TERM", "1s", "systemctl", "is-system-running"])
            .stdin(Stdio::null())
            .output();
        output.is_ok_and(|output| {
            let state = String::from_utf8_lossy(&output.stdout);
            matches!(
                state.trim(),
                "running" | "degraded" | "starting" | "maintenance"
            )
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

pub fn command_available(command: &str) -> bool {
    if command == "crontab" {
        return Command::new(command)
            .arg("-l")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
    }
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn systemd_unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(SYSTEMD_UNIT_NAME)
}

pub fn launchd_plist_path() -> PathBuf {
    PathBuf::from("/Library/LaunchDaemons").join(format!("{LAUNCHD_LABEL}.plist"))
}

fn startup_state_path() -> PathBuf {
    crate::paths::state_dir().join("startup-method")
}

fn has_control_characters(value: &str) -> bool {
    value.chars().any(|ch| ch.is_control())
}

pub fn shell_quote(value: &Path) -> Result<String, String> {
    let value = value.to_string_lossy();
    if has_control_characters(&value) {
        return Err(format!("path contains control characters: {value:?}"));
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn systemd_quote(value: &str) -> Result<String, String> {
    if has_control_characters(value) {
        return Err("systemd value contains a control character".into());
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    ))
}

fn xml_escape(value: &str) -> Result<String, String> {
    if has_control_characters(value) {
        return Err("launchd value contains a control character".into());
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

fn windows_quote(value: &Path) -> Result<String, String> {
    let value = value.to_string_lossy();
    if has_control_characters(&value) {
        return Err(format!("path contains control characters: {value:?}"));
    }
    // This is the CommandLineToArgvW-compatible quoting needed for a single
    // executable argument. The task action itself is passed as one argument
    // to schtasks, not through a shell.
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
        } else if ch == '"' {
            quoted.push_str(&"\\".repeat(slashes * 2 + 1));
            quoted.push('"');
            slashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(slashes));
            quoted.push(ch);
            slashes = 0;
        }
    }
    quoted.push_str(&"\\".repeat(slashes * 2));
    quoted.push('"');
    Ok(quoted)
}

fn is_loopback_host(host: &str, port: u16) -> Result<bool, String> {
    let host = host.trim();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip.is_loopback());
    }
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve configured bind host {host:?}: {e}"))?;
    let mut found = false;
    for address in addresses {
        found = true;
        if !address.ip().is_loopback() {
            return Ok(false);
        }
    }
    Ok(found)
}

/// Centralized safety policy for plaintext operation.
pub fn validate_transport_policy(
    tls_enabled: bool,
    allow_http: bool,
    grpc_loopback: bool,
    http_loopback: bool,
) -> Result<(), String> {
    if !tls_enabled && allow_http && !(grpc_loopback && http_loopback) {
        return Err(
            "SNIP_SYNC_ALLOW_HTTP=true is permitted only when both gRPC and HTTP bind addresses are loopback; set TLS_ENABLED=true for non-loopback deployment"
                .into(),
        );
    }
    if !tls_enabled && !allow_http {
        return Err(
            "TLS termination is required for production. Set TLS_ENABLED=true when a reverse proxy terminates TLS, or set SNIP_SYNC_ALLOW_HTTP=true for loopback development"
                .into(),
        );
    }
    Ok(())
}

pub fn transport_environment() -> Result<TransportEnvironment, String> {
    let config = Config::load().map_err(|e| e.to_string())?;
    let tls_enabled = parse_bool_env(&|name| std::env::var(name).ok(), "TLS_ENABLED")
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    let grpc_loopback = is_loopback_host(&config.grpc_host, config.grpc_port)?;
    let http_loopback = is_loopback_host(&config.http_host, config.http_port)?;
    let mut entries = Vec::new();
    if tls_enabled {
        entries.push(("TLS_ENABLED".into(), "true".into()));
    } else {
        validate_transport_policy(false, true, grpc_loopback, http_loopback)?;
        entries.push(("SNIP_SYNC_ALLOW_HTTP".into(), "true".into()));
    }
    for name in [
        "CONFIG_PATH",
        "SNIP_SYNC_STATE_DIR",
        "DATABASE_URL",
        "PREMADE_DIR",
    ] {
        if let Ok(value) = std::env::var(name) {
            if has_control_characters(&value) {
                return Err(format!("{name} contains a control character"));
            }
            entries.push((name.into(), value));
        }
    }
    Ok(TransportEnvironment { entries })
}

fn environment_lines_systemd(environment: &TransportEnvironment) -> Result<String, String> {
    let mut rendered = String::new();
    for (name, value) in &environment.entries {
        rendered.push_str("Environment=");
        rendered.push_str(&systemd_quote(&format!("{name}={value}"))?);
        rendered.push('\n');
    }
    Ok(rendered)
}

pub fn render_systemd_unit(
    executable: &Path,
    account: &str,
    environment: &TransportEnvironment,
) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| "executable path is not valid UTF-8".to_string())?;
    let exec_start = format!("{} serve", systemd_quote(executable)?);
    if has_control_characters(account) || account.is_empty() {
        return Err("systemd account is empty or contains a control character".into());
    }
    Ok(format!(
        "# {OWNERSHIP_MARKER}\n[Unit]\nDescription=snip-sync gRPC server\nAfter=network.target\n\n[Service]\nType=simple\nUser={account}\nExecStart={exec_start}\n{}Restart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=multi-user.target\n",
        environment_lines_systemd(environment)?
    ))
}

pub fn render_launchd_plist(
    executable: &Path,
    account: &str,
    environment: &TransportEnvironment,
) -> Result<String, String> {
    let executable = xml_escape(
        executable
            .to_str()
            .ok_or_else(|| "executable path is not valid UTF-8".to_string())?,
    )?;
    let account = xml_escape(account)?;
    let mut environment_xml = String::new();
    for (name, value) in &environment.entries {
        environment_xml.push_str(&format!(
            "        <key>{}</key><string>{}</string>\n",
            xml_escape(name)?,
            xml_escape(value)?
        ));
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- {OWNERSHIP_MARKER} -->\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>Label</key><string>{LAUNCHD_LABEL}</string>\n    <key>UserName</key><string>{account}</string>\n    <key>ProgramArguments</key>\n    <array><string>{executable}</string><string>serve</string></array>\n    <key>EnvironmentVariables</key>\n    <dict>\n{environment_xml}    </dict>\n    <key>KeepAlive</key><true/>\n    <key>ProcessType</key><string>Background</string>\n</dict>\n</plist>\n"
    ))
}

fn cron_command(executable: &Path, environment: &TransportEnvironment) -> Result<String, String> {
    let mut command = String::new();
    for (name, value) in &environment.entries {
        // Values are generated from booleans or explicitly supplied paths;
        // shell_quote also rejects control characters.
        let quoted = shell_quote(Path::new(value))?;
        command.push_str(name);
        command.push('=');
        command.push_str(&quoted);
        command.push(' ');
    }
    command.push_str(&shell_quote(executable)?);
    command.push_str(" croncheck");
    Ok(command)
}

pub fn render_cron_block(
    executable: &Path,
    environment: &TransportEnvironment,
) -> Result<String, String> {
    let command = cron_command(executable, environment)?;
    Ok(format!(
        "{CRON_BEGIN}\n@reboot {command}\n*/5 * * * * {command}\n{CRON_END}"
    ))
}

pub fn render_task_commands(
    executable: &Path,
    environment: &TransportEnvironment,
    administrator: bool,
) -> Result<Vec<String>, String> {
    let action = task_action(executable, environment)?;
    let trigger = if administrator { "ONSTART" } else { "ONLOGON" };
    let mut startup = format!(
        "schtasks /Create /TN \"{TASK_STARTUP_NAME}\" /SC {trigger} /TR \"{}\" /F",
        action.replace('"', "\\\"")
    );
    if administrator {
        startup.push_str(" /RU SYSTEM /RL HIGHEST");
    }
    Ok(vec![
        startup,
        format!(
            "schtasks /Create /TN \"{TASK_WATCHDOG_NAME}\" /SC MINUTE /MO 5 /TR \"{}\" /F",
            action.replace('"', "\\\"")
        ),
    ])
}

fn task_action(executable: &Path, environment: &TransportEnvironment) -> Result<String, String> {
    let executable = windows_quote(executable)?;
    let mut command = String::from("cmd.exe /D /S /C \"");
    for (name, value) in &environment.entries {
        if has_control_characters(name) || has_control_characters(value) {
            return Err("Task Scheduler environment contains a control character".into());
        }
        command.push_str("set \\\"");
        command.push_str(name);
        command.push('=');
        command.push_str(value);
        command.push_str("\\\"&&");
    }
    command.push_str(&executable);
    command.push_str(" croncheck\"");
    Ok(command)
}

fn account_name() -> String {
    std::env::var("SUDO_USER")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .unwrap_or_else(|| "snip-sync".into())
}

fn require_non_root_service_account() -> Result<(), String> {
    if is_root() && account_name() == "root" {
        return Err(
            "refusing to register a system service as root; invoke through sudo from the intended non-root account so SUDO_USER is preserved"
                .into(),
        );
    }
    Ok(())
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn exact_elevated_command(method: StartupMethod, executable: &Path) -> Result<String, String> {
    Ok(format!(
        "sudo {} startup install --method {}",
        shell_quote(executable)?,
        method.as_str()
    ))
}

fn require_root(method: StartupMethod, executable: &Path) -> Result<(), String> {
    if !is_root() {
        let command = exact_elevated_command(method, executable)?;
        eprintln!("This startup method requires elevated privileges.");
        eprintln!("Run exactly: {command}");
        return Err("elevated privileges required".into());
    }
    Ok(())
}

fn require_root_for_uninstall(executable: &Path) -> Result<(), String> {
    if !is_root() {
        let command = format!("sudo {} startup uninstall", shell_quote(executable)?);
        eprintln!("This startup removal requires elevated privileges.");
        eprintln!("Run exactly: {command}");
        return Err("elevated privileges required".into());
    }
    Ok(())
}

fn run_checked(program: &str, args: &[&str]) -> Result<Output, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn run_allow_failure(program: &str, args: &[&str]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .output()
        .map(|_| ())
        .map_err(|e| format!("failed to execute {program}: {e}"))
}

fn write_owned(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    fs::write(path, content).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

fn is_owned_file(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .is_some_and(|content| content.contains(OWNERSHIP_MARKER))
}

fn read_crontab() -> Result<String, String> {
    let output = Command::new("crontab")
        .arg("-l")
        .output()
        .map_err(|e| format!("crontab is unavailable: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no crontab") || stderr.contains("no crontab for") {
        Ok(String::new())
    } else {
        Err(format!("crontab -l failed: {}", stderr.trim()))
    }
}

fn replace_cron_block(existing: &str, replacement: Option<&str>) -> String {
    let lines: Vec<&str> = existing.lines().collect();
    let begin = lines.iter().position(|line| *line == CRON_BEGIN);
    let end = begin.and_then(|begin| {
        lines
            .iter()
            .enumerate()
            .skip(begin + 1)
            .find_map(|(index, line)| (*line == CRON_END).then_some(index))
    });
    let mut output = Vec::new();
    match (begin, end) {
        (Some(begin), Some(end)) => {
            output.extend_from_slice(&lines[..begin]);
            output.extend_from_slice(&lines[end + 1..]);
        }
        _ => output.extend(lines),
    }
    if let Some(replacement) = replacement {
        while output.last().is_some_and(|line| line.is_empty()) {
            output.pop();
        }
        if !output.is_empty() {
            output.push("");
        }
        output.extend(replacement.lines());
    }
    let mut result = output.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

fn write_crontab(content: &str) -> Result<(), String> {
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to execute crontab: {e}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "failed to open crontab stdin".to_string())?
        .write_all(content.as_bytes())
        .map_err(|e| format!("failed to write crontab: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed waiting for crontab: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "crontab install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn persist_method(method: StartupMethod) -> Result<(), String> {
    if let Some(parent) = startup_state_path().parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create startup state: {e}"))?;
    }
    fs::write(startup_state_path(), method.as_str())
        .map_err(|e| format!("failed to record startup method: {e}"))
}

fn installed_method() -> Option<StartupMethod> {
    let value = fs::read_to_string(startup_state_path()).ok()?;
    value.trim().parse().ok()
}

pub fn install(method_arg: StartupMethodArg, executable: &Path) -> Result<(), String> {
    let method = resolve_method(method_arg)?;
    if method == StartupMethod::Direct {
        return Err("direct is an internal unmanaged state and cannot be installed".into());
    }
    if matches!(method, StartupMethod::Systemd | StartupMethod::Launchd) {
        require_root(method, executable)?;
    }
    if method == StartupMethod::Cron && !command_available("crontab") {
        return Err("cron startup requested but crontab is unavailable".into());
    }
    crate::bootstrap::ensure_layout()?;
    crate::bootstrap::ensure_config_file()?;
    let environment = transport_environment()?;
    match method {
        StartupMethod::Systemd => install_systemd(executable, &environment)?,
        StartupMethod::Launchd => install_launchd(executable, &environment)?,
        StartupMethod::Cron => install_cron(executable, &environment)?,
        StartupMethod::TaskScheduler => install_task_scheduler(executable, &environment)?,
        StartupMethod::Direct => unreachable!(),
    }
    persist_method(method)?;
    println!("Installed snip-sync startup using {}.", method.as_str());
    Ok(())
}

fn install_systemd(executable: &Path, environment: &TransportEnvironment) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("systemd startup installation is supported only on Linux".into());
    }
    require_non_root_service_account()?;
    let unit = render_systemd_unit(executable, &account_name(), environment)?;
    let path = systemd_unit_path();
    write_owned(&path, &unit)?;
    run_checked("systemctl", &["daemon-reload"])?;
    run_checked("systemctl", &["enable", SYSTEMD_UNIT_NAME])?;
    run_checked("systemctl", &["restart", SYSTEMD_UNIT_NAME])?;
    let config = Config::load().map_err(|e| e.to_string())?;
    wait_for_health(&config, Duration::from_secs(10))?;
    println!("Unit: {}", path.display());
    println!("Status: systemctl status {SYSTEMD_UNIT_NAME}");
    Ok(())
}

fn install_launchd(executable: &Path, environment: &TransportEnvironment) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("launchd startup installation is supported only on macOS".into());
    }
    require_non_root_service_account()?;
    let path = launchd_plist_path();
    let plist = render_launchd_plist(executable, &account_name(), environment)?;
    write_owned(&path, &plist)?;
    let path_string = path
        .to_str()
        .ok_or_else(|| "launchd plist path is not valid UTF-8".to_string())?;
    let target = format!("system/{LAUNCHD_LABEL}");
    let _ = run_allow_failure("launchctl", &["bootout", &target]);
    run_checked("launchctl", &["bootstrap", "system", path_string])?;
    run_checked("launchctl", &["enable", &format!("system/{LAUNCHD_LABEL}")])?;
    run_checked(
        "launchctl",
        &["kickstart", "-k", &format!("system/{LAUNCHD_LABEL}")],
    )?;
    let config = Config::load().map_err(|e| e.to_string())?;
    wait_for_health(&config, Duration::from_secs(10))?;
    println!("Plist: {}", path.display());
    println!("Status: launchctl print system/{LAUNCHD_LABEL}");
    Ok(())
}

fn install_cron(executable: &Path, environment: &TransportEnvironment) -> Result<(), String> {
    let block = render_cron_block(executable, environment)?;
    let existing = read_crontab()?;
    write_crontab(&replace_cron_block(&existing, Some(&block)))
}

fn install_task_scheduler(
    executable: &Path,
    environment: &TransportEnvironment,
) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Task Scheduler installation is supported only on Windows".into());
    }
    let administrator = is_windows_administrator();
    let action = task_action(executable, environment)?;
    let trigger = if administrator { "ONSTART" } else { "ONLOGON" };
    let mut startup_args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        TASK_STARTUP_NAME.to_string(),
        "/SC".to_string(),
        trigger.to_string(),
        "/TR".to_string(),
        action.clone(),
        "/F".to_string(),
    ];
    if administrator {
        startup_args.extend([
            "/RU".to_string(),
            "SYSTEM".to_string(),
            "/RL".to_string(),
            "HIGHEST".to_string(),
        ]);
    }
    let watchdog_args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        TASK_WATCHDOG_NAME.to_string(),
        "/SC".to_string(),
        "MINUTE".to_string(),
        "/MO".to_string(),
        "5".to_string(),
        "/TR".to_string(),
        action,
        "/F".to_string(),
    ];
    for args in [startup_args, watchdog_args] {
        run_checked(
            "schtasks",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_administrator() -> bool {
    Command::new("net")
        .args(["session"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(windows))]
fn is_windows_administrator() -> bool {
    false
}

pub fn instructions(method_arg: StartupMethodArg, executable: &Path) -> Result<(), String> {
    let method = resolve_method(method_arg)?;
    let environment = transport_environment()?;
    println!("Selected startup method: {}", method.as_str());
    match method {
        StartupMethod::Systemd => {
            let path = systemd_unit_path();
            println!("\nRun:\n  {}", exact_elevated_command(method, executable)?);
            println!(
                "\nFile: {}\n\n{}",
                path.display(),
                render_systemd_unit(executable, &account_name(), &environment)?
            );
        }
        StartupMethod::Launchd => {
            let path = launchd_plist_path();
            println!("\nRun:\n  {}", exact_elevated_command(method, executable)?);
            println!(
                "\nFile: {}\n\n{}",
                path.display(),
                render_launchd_plist(executable, &account_name(), &environment)?
            );
        }
        StartupMethod::Cron => {
            println!("\nRun: crontab -e");
            println!(
                "\nAdd exactly:\n{}",
                render_cron_block(executable, &environment)?
            );
        }
        StartupMethod::TaskScheduler => {
            println!("\nRun these commands in an elevated prompt when installing for all users:");
            for command in render_task_commands(executable, &environment, true)? {
                println!("{command}");
            }
            println!("\nNon-administrator fallback commands:");
            for command in render_task_commands(executable, &environment, false)? {
                println!("{command}");
            }
        }
        StartupMethod::Direct => unreachable!(),
    }
    Ok(())
}

pub fn uninstall(executable: &Path) -> Result<(), String> {
    let mut changed = false;
    #[cfg(target_os = "linux")]
    {
        if is_owned_file(&systemd_unit_path()) {
            require_root_for_uninstall(executable)?;
            let _ = run_allow_failure("systemctl", &["disable", "--now", SYSTEMD_UNIT_NAME]);
            fs::remove_file(systemd_unit_path())
                .map_err(|e| format!("failed to remove systemd unit: {e}"))?;
            let _ = run_allow_failure("systemctl", &["daemon-reload"]);
            changed = true;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if is_owned_file(&launchd_plist_path()) {
            require_root_for_uninstall(executable)?;
            let _ = run_allow_failure(
                "launchctl",
                &["bootout", &format!("system/{LAUNCHD_LABEL}")],
            );
            fs::remove_file(launchd_plist_path())
                .map_err(|e| format!("failed to remove launchd plist: {e}"))?;
            changed = true;
        }
    }
    #[cfg(any(unix, target_os = "linux", target_os = "macos"))]
    if command_available("crontab") {
        let existing = read_crontab()?;
        let updated = replace_cron_block(&existing, None);
        if updated != existing {
            write_crontab(&updated)?;
            changed = true;
        }
    }
    #[cfg(windows)]
    {
        for name in [TASK_STARTUP_NAME, TASK_WATCHDOG_NAME] {
            let _ = run_allow_failure("schtasks", &["/Delete", "/TN", name, "/F"]);
            changed = true;
        }
    }
    if startup_state_path().exists() {
        let _ = fs::remove_file(startup_state_path());
        changed = true;
    }
    if changed {
        println!("Removed snip-sync-owned startup registration.");
    } else {
        println!("No snip-sync startup registration found.");
    }
    Ok(())
}

pub fn check_health(http_host: &str, http_port: u16) -> bool {
    let address = match (http_host, http_port).to_socket_addrs() {
        Ok(mut addresses) => match addresses.next() {
            Some(address) => address,
            None => return false,
        },
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET /health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = [0u8; 4096];
    let bytes_read = match stream.read(&mut response) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let response = String::from_utf8_lossy(&response[..bytes_read]);
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

fn wait_for_health(config: &Config, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check_health(&config.http_host, config.http_port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "snip-sync did not become healthy at {}:{} within {} seconds",
        config.http_host,
        config.http_port,
        timeout.as_secs()
    ))
}

fn server_is_running() -> Result<bool, String> {
    match crate::server_lock::ServerLock::try_acquire(&crate::paths::state_dir()) {
        Ok(guard) => {
            drop(guard);
            Ok(false)
        }
        Err(crate::server_lock::ServerLockError::Busy { .. }) => Ok(true),
        Err(error) => Err(error.to_string()),
    }
}

/// Read-only lifecycle probe used by `update --dry-run`. It deliberately does
/// not call `ServerLock::try_acquire`, because that API creates/publishes lock
/// metadata when the server is absent.
fn server_is_running_read_only() -> bool {
    let path = crate::server_lock::server_lock_path(&crate::paths::state_dir());
    let Some(owner) = crate::server_lock::read_owner(&path) else {
        return false;
    };
    if !crate::process::is_running(owner.pid) {
        return false;
    }
    owner.start_token.as_ref().is_none_or(|expected| {
        crate::process::get_process_start_token(owner.pid).is_none_or(|actual| actual == *expected)
    })
}

pub fn classify_update_lifecycle(
    installed: Option<StartupMethod>,
    running: bool,
) -> UpdateLifecycle {
    match (installed, running) {
        (Some(method), true) => UpdateLifecycle::ManagedRunning(method),
        (Some(method), false) => UpdateLifecycle::ManagedStopped(method),
        (None, true) => UpdateLifecycle::DirectRunning,
        (None, false) => UpdateLifecycle::NotRunning,
    }
}

/// Capture the server state before an executable update. A stopped installed
/// service is deliberately distinct from an absent service so an update never
/// starts a server solely because it was requested.
pub fn update_lifecycle() -> Result<UpdateLifecycle, String> {
    Ok(classify_update_lifecycle(
        installed_method(),
        server_is_running_read_only(),
    ))
}

fn restart_systemd() -> Result<(), String> {
    run_checked("systemctl", &["restart", SYSTEMD_UNIT_NAME])?;
    Ok(())
}

fn restart_launchd() -> Result<(), String> {
    let target = format!("system/{LAUNCHD_LABEL}");
    run_checked("launchctl", &["kickstart", "-k", &target])?;
    Ok(())
}

fn spawn_server(executable: &Path) -> Result<(), String> {
    Command::new(executable)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to start detached server: {e}"))
}

/// Restart only a server that was running when the update began. Errors are
/// intentionally returned after the binary replacement; callers can report a
/// partial activation without rolling the verified binary back.
pub fn restart_after_update(
    executable: &Path,
    lifecycle: UpdateLifecycle,
    force: bool,
) -> Result<(), String> {
    let config = || Config::load().map_err(|e| e.to_string());
    match lifecycle {
        UpdateLifecycle::ManagedRunning(StartupMethod::Systemd) => {
            restart_systemd()?;
            wait_for_health(&config()?, Duration::from_secs(10))
        }
        UpdateLifecycle::ManagedRunning(StartupMethod::Launchd) => {
            restart_launchd()?;
            wait_for_health(&config()?, Duration::from_secs(10))
        }
        UpdateLifecycle::ManagedRunning(StartupMethod::Cron | StartupMethod::TaskScheduler) => {
            let mut stop = Command::new(executable);
            stop.arg("stop");
            if force {
                stop.arg("--force");
            }
            let status = stop
                .status()
                .map_err(|e| format!("failed to stop server for managed restart: {e}"))?;
            if !status.success() {
                return Err("managed restart could not stop the running server".into());
            }
            spawn_croncheck(executable)?;
            wait_for_health(&config()?, Duration::from_secs(10))
        }
        UpdateLifecycle::DirectRunning | UpdateLifecycle::ManagedRunning(StartupMethod::Direct) => {
            let mut stop = Command::new(executable);
            stop.arg("stop");
            if force {
                stop.arg("--force");
            }
            let status = stop
                .status()
                .map_err(|e| format!("failed to stop server for restart: {e}"))?;
            if !status.success() {
                return Err("direct restart could not stop the running server".into());
            }
            spawn_server(executable)?;
            wait_for_health(&config()?, Duration::from_secs(10))
        }
        UpdateLifecycle::ManagedStopped(_) | UpdateLifecycle::NotRunning => Ok(()),
    }
}

fn spawn_croncheck(executable: &Path) -> Result<(), String> {
    Command::new(executable)
        .arg("croncheck")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to start detached croncheck: {e}"))
}

/// Restart through an installed manager. `Ok(false)` means the caller should
/// use the existing unmanaged stop + foreground serve path.
pub fn restart_if_managed(executable: &Path, force: bool) -> Result<bool, String> {
    let Some(method) = installed_method() else {
        return Ok(false);
    };
    match method {
        StartupMethod::Systemd => {
            let state = Command::new("systemctl")
                .args(["is-active", "--quiet", SYSTEMD_UNIT_NAME])
                .status()
                .map_err(|e| format!("failed to inspect systemd service: {e}"))?;
            let action = if state.success() { "restart" } else { "start" };
            run_checked("systemctl", &[action, SYSTEMD_UNIT_NAME])?;
            let config = Config::load().map_err(|e| e.to_string())?;
            wait_for_health(&config, Duration::from_secs(10))?;
            Ok(true)
        }
        StartupMethod::Launchd => {
            let target = format!("system/{LAUNCHD_LABEL}");
            let loaded = Command::new("launchctl")
                .args(["print", &target])
                .status()
                .map_err(|e| format!("failed to inspect launchd job: {e}"))?
                .success();
            if loaded {
                run_checked("launchctl", &["kickstart", "-k", &target])?;
            } else {
                let path = launchd_plist_path();
                let path = path
                    .to_str()
                    .ok_or_else(|| "plist path is not UTF-8".to_string())?;
                run_checked("launchctl", &["bootstrap", "system", path])?;
            }
            let config = Config::load().map_err(|e| e.to_string())?;
            wait_for_health(&config, Duration::from_secs(10))?;
            Ok(true)
        }
        StartupMethod::Cron | StartupMethod::TaskScheduler => {
            if server_is_running()? {
                let mut stop = Command::new(executable);
                stop.arg("stop");
                if force {
                    stop.arg("--force");
                }
                let status = stop
                    .status()
                    .map_err(|e| format!("failed to run stop: {e}"))?;
                if !status.success() {
                    return Err("managed restart could not stop the running server".into());
                }
            }
            spawn_croncheck(executable)?;
            Ok(true)
        }
        StartupMethod::Direct => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_lifecycle_decision_table_preserves_stopped_state() {
        assert_eq!(
            classify_update_lifecycle(Some(StartupMethod::Systemd), true),
            UpdateLifecycle::ManagedRunning(StartupMethod::Systemd)
        );
        assert_eq!(
            classify_update_lifecycle(Some(StartupMethod::TaskScheduler), false),
            UpdateLifecycle::ManagedStopped(StartupMethod::TaskScheduler)
        );
        assert_eq!(
            classify_update_lifecycle(None, true),
            UpdateLifecycle::DirectRunning
        );
        assert_eq!(
            classify_update_lifecycle(None, false),
            UpdateLifecycle::NotRunning
        );
    }

    fn loopback_env() -> TransportEnvironment {
        TransportEnvironment {
            entries: vec![("SNIP_SYNC_ALLOW_HTTP".into(), "true".into())],
        }
    }

    #[test]
    fn systemd_requires_actual_runtime_markers() {
        assert!(!systemd_environment_is_running(false, Some("init"), false));
        assert!(systemd_environment_is_running(true, Some("init"), false));
        assert!(systemd_environment_is_running(
            false,
            Some("systemd"),
            false
        ));
        assert!(systemd_environment_is_running(false, Some("init"), true));
    }

    fn systemd_environment_is_running(
        run_dir_exists: bool,
        pid_one_comm: Option<&str>,
        bounded_probe_succeeded: bool,
    ) -> bool {
        run_dir_exists || pid_one_comm == Some("systemd") || bounded_probe_succeeded
    }

    #[test]
    fn shell_and_windows_quoting_rejects_controls() {
        assert_eq!(
            shell_quote(Path::new("/tmp/a'b")),
            Ok("'/tmp/a'\\''b'".into())
        );
        assert!(shell_quote(Path::new("/tmp/a\n")).is_err());
        assert!(windows_quote(Path::new("C:\\tmp\\a\n")).is_err());
    }

    #[test]
    fn renders_exact_supervisor_records() {
        let executable = Path::new("/opt/snip-sync");
        let systemd = render_systemd_unit(executable, "alice", &loopback_env()).unwrap();
        assert!(systemd.contains("ExecStart=\"/opt/snip-sync\" serve"));
        assert!(systemd.contains("RestartSec=2"));
        assert!(
            render_launchd_plist(executable, "alice", &loopback_env())
                .unwrap()
                .contains("com.eggstack.snip-sync")
        );
        let cron = render_cron_block(executable, &loopback_env()).unwrap();
        assert!(cron.contains("@reboot SNIP_SYNC_ALLOW_HTTP='true' '/opt/snip-sync' croncheck"));
        assert!(!cron.contains(" serve"));
        let task = render_task_commands(executable, &loopback_env(), true).unwrap();
        assert!(task.iter().all(|line| line.contains("croncheck")));
    }

    #[test]
    fn cron_replacement_is_idempotent_and_preserves_unrelated_entries() {
        let existing = "MAILTO=alice\n\n# old\n";
        let block = render_cron_block(Path::new("/tmp/snip-sync"), &loopback_env()).unwrap();
        let once = replace_cron_block(existing, Some(&block));
        let twice = replace_cron_block(&once, Some(&block));
        assert_eq!(once, twice);
        assert!(replace_cron_block(&twice, None).contains("MAILTO=alice"));
        assert!(!replace_cron_block(&twice, None).contains(CRON_BEGIN));
        let malformed = format!("MAILTO=alice\n{CRON_BEGIN}\n# unrelated trailing entry");
        assert!(replace_cron_block(&malformed, None).contains("# unrelated trailing entry"));
    }

    #[test]
    fn transport_policy_only_allows_plaintext_loopback() {
        assert!(validate_transport_policy(false, true, true, true).is_ok());
        assert!(validate_transport_policy(true, false, false, false).is_ok());
        assert!(validate_transport_policy(false, true, false, true).is_err());
        assert!(validate_transport_policy(false, false, true, true).is_err());
    }
}
