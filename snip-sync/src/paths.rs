use std::fmt;
use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("snip-sync")
}

pub fn config_path() -> PathBuf {
    if let Ok(val) = std::env::var("CONFIG_PATH") {
        return PathBuf::from(val);
    }
    config_dir().join("config.toml")
}

pub fn data_dir() -> PathBuf {
    #[allow(clippy::redundant_closure)]
    dirs::data_dir()
        .unwrap_or_else(|| config_dir())
        .join("snip-sync")
}

pub fn state_dir() -> PathBuf {
    #[allow(clippy::redundant_closure)]
    dirs::state_dir()
        .unwrap_or_else(|| data_dir())
        .join("snip-sync")
}

pub fn cert_dir() -> PathBuf {
    config_dir().join("certs")
}

pub fn pid_path() -> PathBuf {
    state_dir().join("snip-sync.pid")
}

pub fn default_db_path() -> PathBuf {
    config_dir().join("snippets.db")
}

pub fn default_premade_dir() -> PathBuf {
    data_dir().join("premade-libraries")
}

pub struct Paths {
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cert_dir: PathBuf,
    pub pid_path: PathBuf,
    pub db_path: PathBuf,
    pub premade_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let config_dir = config_dir();
        let config_path = config_path();
        let data_dir = data_dir();
        let state_dir = state_dir();
        let cert_dir = cert_dir();
        let pid_path = pid_path();
        let db_path = default_db_path();
        let premade_dir = default_premade_dir();

        Self {
            config_dir,
            config_path,
            data_dir,
            state_dir,
            cert_dir,
            pid_path,
            db_path,
            premade_dir,
        }
    }

    pub fn print(&self) {
        println!("{}", self);
    }
}

impl fmt::Display for Paths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "config_dir:      {}", self.config_dir.display())?;
        writeln!(f, "config_path:     {}", self.config_path.display())?;
        writeln!(f, "data_dir:        {}", self.data_dir.display())?;
        writeln!(f, "state_dir:       {}", self.state_dir.display())?;
        writeln!(f, "cert_dir:        {}", self.cert_dir.display())?;
        writeln!(f, "pid_path:        {}", self.pid_path.display())?;
        writeln!(f, "db_path:         {}", self.db_path.display())?;
        writeln!(f, "premade_dir:     {}", self.premade_dir.display())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_not_empty() {
        let d = config_dir();
        assert!(d.ends_with("snip-sync"));
    }

    #[test]
    fn test_config_path_respects_env() {
        unsafe {
            std::env::set_var("CONFIG_PATH", "/tmp/test-config.toml");
        }
        assert_eq!(config_path(), PathBuf::from("/tmp/test-config.toml"));
        unsafe {
            std::env::remove_var("CONFIG_PATH");
        }
    }

    #[test]
    fn test_paths_resolve() {
        let paths = Paths::resolve();
        assert!(paths.config_dir.ends_with("snip-sync"));
        assert!(paths.config_path.ends_with("config.toml"));
        assert!(paths.pid_path.ends_with("snip-sync.pid"));
        assert!(paths.db_path.ends_with("snippets.db"));
    }

    #[test]
    fn test_display() {
        let paths = Paths::resolve();
        let display = format!("{}", paths);
        assert!(display.contains("config_dir:"));
        assert!(display.contains("pid_path:"));
    }
}
