//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and cloud sync with end-to-end encryption.

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
use utils::config::get_snippets_path;

pub struct SnippetData {
    pub descriptions: Vec<String>,
    pub commands: Vec<String>,
    pub tags: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub favorites: Vec<bool>,
}

pub enum ProcessResult {
    Cancel,
    Continue,
    Done(String),
}

static CONFIG_PATH: LazyLock<PathBuf> = LazyLock::new(get_snippets_path);

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime"));

#[cfg(unix)]
fn setup_signal_handler() {
    use signal_hook::flag;

    let terminate = ui::get_terminate();

    flag::register(signal_hook::consts::signal::SIGINT, terminate.clone())
        .expect("Failed to set Ctrl+C handler");
    flag::register(signal_hook::consts::signal::SIGTERM, terminate)
        .expect("Failed to set SIGTERM handler");
}

#[cfg(windows)]
fn setup_signal_handler() {
    // Windows: Ctrl+C is handled by crossterm's event loop
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
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        csv: bool,
    },
    /// Run a snippet via TUI selection (r)
    #[command(alias = "r")]
    Run {
        #[arg(short, long)]
        filter: Option<String>,
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
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync: bool,
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Edit the config file in $EDITOR (e)
    #[command(alias = "e")]
    Edit {
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Show keybindings
    #[command(alias = "k")]
    Keybindings,
    /// Sync snippets with server
    #[command(alias = "y")]
    Sync {
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
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
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
        #[arg(long, action = clap::ArgAction::SetTrue)]
        force: bool,
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

fn dispatch_command(cli: Commands) -> SnipResult<()> {
    match cli {
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
            json,
            csv,
        } => {
            let format = if json {
                commands::list_cmd::ListFormat::Json
            } else if csv {
                commands::list_cmd::ListFormat::Csv
            } else {
                commands::list_cmd::ListFormat::Default
            };
            commands::list_cmd::run(filter, config, library, format)?;
        }
        Commands::Run {
            filter,
            sync,
            library,
        } => {
            commands::run_cmd::run(filter, sync, library, &RUNTIME)?;
        }
        Commands::Clip {
            filter,
            sync,
            library,
        } => {
            commands::clip_cmd::run(filter, sync, library, None, &RUNTIME)?;
        }
        Commands::Search {
            filter,
            sync,
            library,
        } => {
            commands::search_cmd::run(filter, sync, library, None, &RUNTIME)?;
        }
        Commands::Edit { library } => {
            commands::edit_cmd::run(library, None)?;
        }
        Commands::Keybindings => {
            commands::keybindings_cmd::run()?;
        }
        Commands::Sync {
            library,
            servers,
            non_interactive,
            push_only,
            pull_only,
            dry_run,
        } => {
            let options = commands::sync_cmd::SyncOptions {
                library,
                servers,
                non_interactive,
                push_only,
                pull_only,
                dry_run,
            };
            commands::sync_cmd::run(options, &RUNTIME)?;
        }
        Commands::Cron { interval } => {
            commands::cron_cmd::run(interval)?;
        }
        Commands::Register { server, force } => {
            commands::register_cmd::run(server, force, &RUNTIME)?;
        }
        Commands::Library { command } => match command {
            LibraryCommands::List => commands::library_cmd::run_list()?,
            LibraryCommands::Create { name } => commands::library_cmd::run_create(name)?,
            LibraryCommands::Delete { name, force } => {
                commands::library_cmd::run_delete(name, force)?
            }
            LibraryCommands::SetPrimary { name } => commands::library_cmd::run_set_primary(name)?,
            LibraryCommands::Show { name } => commands::library_cmd::run_show(name)?,
        },
        Commands::Premade { command } => match command {
            PremadeCommands::List => commands::premade_cmd::run_list(&RUNTIME)?,
            PremadeCommands::Get { name } => {
                let all = name.as_ref().is_some_and(|n| n == "all");
                commands::premade_cmd::run_get(name, all, &RUNTIME)?;
            }
            PremadeCommands::Sync => commands::premade_cmd::run_sync(&RUNTIME)?,
        },
    }
    Ok(())
}

fn main() -> SnipResult<()> {
    setup_panic_handler();
    setup_signal_handler();
    init_default_logging();
    log_startup_info();

    let cli = Cli::parse();
    dispatch_command(cli.command)?;

    log_shutdown_info();
    Ok(())
}
