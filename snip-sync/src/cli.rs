use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "snip-sync",
    version,
    about = "gRPC snippet sync server for snp"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the sync server (default if no subcommand given)
    Serve,
    /// Initialize config, directories, and dev certificates
    Init {
        #[arg(long)]
        force_cert: bool,
        #[arg(long)]
        skip_cert: bool,
    },
    /// Generate self-signed dev certificates
    Cert {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Open config file in $EDITOR
    Edit,
    /// Stop a running server
    Stop {
        #[arg(long)]
        force: bool,
    },
    /// Restart the server (stop + serve)
    Restart {
        #[arg(long)]
        force: bool,
    },
    /// Install, inspect, or remove boot-time startup registration
    Startup {
        #[command(subcommand)]
        command: StartupCommand,
    },
    /// Check for and install an update from crates.io when Cargo-managed
    Update {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        locked: bool,
    },
    /// Internal Windows self-replacement helper.
    #[command(name = "__self-replace", hide = true)]
    SelfReplace {
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        restart: bool,
        #[arg(long)]
        managed_restart: bool,
    },
    /// Health check; start server if unhealthy/down
    Croncheck {
        #[arg(long)]
        verbose: bool,
    },
    /// Print resolved paths
    Paths {
        #[arg(long)]
        json: bool,
    },
    /// Generate shell completions
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print version
    Version,
}

#[derive(Subcommand)]
pub enum StartupCommand {
    /// Install startup registration using the selected or detected manager
    Install {
        #[arg(long, value_enum, default_value_t = StartupMethodArg::Auto)]
        method: StartupMethodArg,
    },
    /// Print exact manual startup commands and rendered configuration
    Instructions {
        #[arg(long, value_enum, default_value_t = StartupMethodArg::Auto)]
        method: StartupMethodArg,
    },
    /// Remove snip-sync-owned startup registration
    Uninstall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StartupMethodArg {
    Auto,
    Systemd,
    Launchd,
    Cron,
    #[value(name = "task-scheduler")]
    TaskScheduler,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_struct_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn test_parse_serve() {
        let cli = Cli::try_parse_from(["snip-sync", "serve"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Serve)));
    }

    #[test]
    fn test_parse_no_subcommand() {
        let cli = Cli::try_parse_from(["snip-sync"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_parse_init() {
        let cli = Cli::try_parse_from(["snip-sync", "init", "--force-cert"]).unwrap();
        match cli.command {
            Some(Command::Init {
                force_cert,
                skip_cert,
            }) => {
                assert!(force_cert);
                assert!(!skip_cert);
            }
            _ => panic!("Expected Init command"),
        }
    }

    #[test]
    fn test_parse_cert() {
        let cli = Cli::try_parse_from(["snip-sync", "cert", "--force"]).unwrap();
        match cli.command {
            Some(Command::Cert { force, out_dir }) => {
                assert!(force);
                assert!(out_dir.is_none());
            }
            _ => panic!("Expected Cert command"),
        }
    }

    #[test]
    fn test_parse_stop() {
        let cli = Cli::try_parse_from(["snip-sync", "stop"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Stop { force: false })));
    }

    #[test]
    fn test_parse_startup_commands() {
        let cli = Cli::try_parse_from([
            "snip-sync",
            "startup",
            "install",
            "--method",
            "task-scheduler",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Startup {
                command: StartupCommand::Install {
                    method: StartupMethodArg::TaskScheduler
                }
            })
        ));
    }

    #[test]
    fn test_parse_version() {
        let cli = Cli::try_parse_from(["snip-sync", "version"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Version)));
    }

    #[test]
    fn test_parse_update() {
        let cli = Cli::try_parse_from(["snip-sync", "update", "--dry-run", "--locked"]).unwrap();
        match cli.command {
            Some(Command::Update { dry_run, locked }) => {
                assert!(dry_run);
                assert!(locked);
            }
            _ => panic!("Expected Update command"),
        }
    }
}
