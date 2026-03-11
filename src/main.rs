use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};

mod clipboard;
mod commands;
mod config;
mod encryption;
mod error;
mod library;
mod logging;
mod sync;
mod sync_commands;
mod ui;
mod utils;

use error::SnipResult;
use logging::{init_default_logging, log_shutdown_info, log_startup_info, setup_panic_handler};

pub type SnippetData = (
    Vec<String>,
    Vec<String>,
    Vec<Vec<String>>,
    Vec<Vec<String>>,
    Vec<bool>,
);

pub enum ProcessResult {
    Cancel,
    Continue,
    Done(String),
}

static CONFIG_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let cfg_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".config")));
    cfg_dir.join("snp/snippets.toml")
});

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime"));

fn setup_signal_handler() {
    use signal_hook::low_level;

    let terminate = ui::get_terminate();
    let terminate_for_int = terminate.clone();
    let terminate_for_term = terminate.clone();

    unsafe {
        low_level::register(signal_hook::consts::signal::SIGINT, move || {
            terminate_for_int.store(true, std::sync::atomic::Ordering::SeqCst);
            log_shutdown_info();
            std::process::exit(0);
        })
        .expect("Failed to set Ctrl+C handler");

        low_level::register(signal_hook::consts::signal::SIGTERM, move || {
            terminate_for_term.store(true, std::sync::atomic::Ordering::SeqCst);
            log_shutdown_info();
            std::process::exit(0);
        })
        .expect("Failed to set SIGTERM handler");
    }
}

#[derive(Debug, Parser)]
#[command(name = "snp", about = "Snippet manager", version = env!("CARGO_PKG_VERSION"), disable_version_flag = true)]
struct Cli {
    #[arg(short = 'v', long, action = clap::ArgAction::Version)]
    version: (),
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show version (v)
    #[command(alias = "v")]
    Version,
    /// Create a new snippet (n)
    #[command(alias = "n")]
    New {
        #[arg(default_value = "")]
        command: String,
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        tags: bool,
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        multiline: bool,
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// List all snippets (l)
    #[command(alias = "l")]
    List {
        #[arg(short, long)]
        filter: Option<String>,
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Run a snippet via TUI selection (r)
    #[command(alias = "r")]
    Run {
        #[arg(short, long)]
        filter: Option<String>,
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync: bool,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Copy a snippet to clipboard via TUI selection (c)
    #[command(alias = "c")]
    Clip {
        #[arg(short, long)]
        filter: Option<String>,
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync: bool,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Search for a snippet via TUI selection (s)
    #[command(alias = "s")]
    Search {
        #[arg(short, long)]
        filter: Option<String>,
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync: bool,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Edit the config file in $EDITOR (e)
    #[command(alias = "e")]
    Edit {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Custom keybindings configuration
    #[command(alias = "k")]
    Keybindings {
        #[arg(long)]
        show: bool,
        #[arg(long)]
        set: Option<String>,
    },
    /// Sync snippets with server
    #[command(alias = "y")]
    Sync {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        library: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        servers: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        non_interactive: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        push_only: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        pull_only: bool,
    },
    /// Setup automatic sync with cron
    Cron {
        #[arg(short, long, default_value = "15")]
        interval: u32,
    },
    /// Register a new sync account
    #[command(alias = "reg")]
    Register {
        #[arg(long, default_value = "http://localhost:50051")]
        server: String,
    },
    /// Manage snippet libraries
    #[command(alias = "lib")]
    Library {
        #[command(subcommand)]
        command: LibraryCommands,
    },
    /// Browse and download premade snippet libraries
    #[command(alias = "p")]
    Premade {
        #[command(subcommand)]
        command: PremadeCommands,
    },
}

#[derive(Debug, Subcommand)]
enum LibraryCommands {
    /// List all libraries
    #[command(alias = "l")]
    List,
    /// Create a new library
    #[command(alias = "c")]
    Create { name: String },
    /// Delete a library
    #[command(alias = "d")]
    Delete {
        name: String,
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Set primary library
    #[command(alias = "p")]
    SetPrimary { name: String },
    /// Show library details
    #[command(alias = "s")]
    Show { name: Option<String> },
}

#[derive(Debug, Subcommand)]
enum PremadeCommands {
    /// List available premade libraries from server
    #[command(alias = "l")]
    List,
    /// Download and install a premade library (or all with 'all')
    Get { name: Option<String> },
    /// Sync all premade libraries with server (download missing)
    #[command(alias = "s")]
    Sync,
}

fn main() -> SnipResult<()> {
    setup_panic_handler();
    setup_signal_handler();
    init_default_logging();
    log_startup_info();

    let cli = Cli::parse();

    match cli.command {
        Commands::Version => {
            println!("snp {}", env!("CARGO_PKG_VERSION"));
        }
        Commands::New {
            command,
            tags,
            multiline,
            config,
            library,
        } => {
            commands::new_cmd::run(command, tags, multiline, config, library)?;
        }
        Commands::List {
            filter,
            config,
            library,
        } => {
            commands::list_cmd::run(filter, config, library)?;
        }
        Commands::Run {
            filter,
            sync,
            library,
            config,
        } => {
            commands::run_cmd::run(filter, sync, library, config, &RUNTIME)?;
        }
        Commands::Clip {
            filter,
            sync,
            library,
            config,
        } => {
            commands::clip_cmd::run(filter, sync, library, config, &RUNTIME)?;
        }
        Commands::Search {
            filter,
            sync,
            library,
            config,
        } => {
            commands::search_cmd::run(filter, sync, library, config, &RUNTIME)?;
        }
        Commands::Edit { config, library } => {
            commands::edit_cmd::run(library, config)?;
        }
        Commands::Keybindings { show, set } => {
            commands::keybindings_cmd::run(show, set)?;
        }
        Commands::Sync {
            config,
            library,
            servers,
            non_interactive,
            push_only,
            pull_only,
        } => {
            commands::sync_cmd::run(
                config,
                library,
                servers,
                non_interactive,
                push_only,
                pull_only,
                &RUNTIME,
            )?;
        }
        Commands::Cron { interval } => {
            commands::cron_cmd::run(interval)?;
        }
        Commands::Register { server } => {
            commands::register_cmd::run(server, &RUNTIME)?;
        }
        Commands::Library { command } => match command {
            LibraryCommands::List => {
                commands::library_cmd::run_list()?;
            }
            LibraryCommands::Create { name } => {
                commands::library_cmd::run_create(name)?;
            }
            LibraryCommands::Delete { name, force } => {
                commands::library_cmd::run_delete(name, force)?;
            }
            LibraryCommands::SetPrimary { name } => {
                commands::library_cmd::run_set_primary(name)?;
            }
            LibraryCommands::Show { name } => {
                commands::library_cmd::run_show(name)?;
            }
        },
        Commands::Premade { command } => match command {
            PremadeCommands::List => {
                commands::premade_cmd::run_list(&RUNTIME)?;
            }
            PremadeCommands::Get { name } => {
                let all = name.as_ref().is_some_and(|n| n == "all");
                commands::premade_cmd::run_get(name, all, &RUNTIME)?;
            }
            PremadeCommands::Sync => {
                commands::premade_cmd::run_sync(&RUNTIME)?;
            }
        },
    }

    log_shutdown_info();
    Ok(())
}
