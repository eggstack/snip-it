//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and cloud sync with end-to-end encryption.

use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

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

/// Aggregated data for all snippets passed to the TUI selector.
///
/// Contains parallel vectors of snippet metadata where index `i` corresponds
/// to the same snippet across all fields.
pub struct SnippetData {
    pub descriptions: Vec<String>,
    pub commands: Vec<String>,
    pub tags: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub favorites: Vec<bool>,
}

/// Result of processing a snippet selection from the TUI.
pub enum ProcessResult {
    /// User cancelled the selection.
    Cancel,
    /// No snippet was selected; continue to next prompt.
    Continue,
    /// A snippet command was selected; contains the expanded command string.
    Done(String),
}

static CONFIG_PATH: LazyLock<PathBuf> = LazyLock::new(get_snippets_path);

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("Failed to create async runtime: {}. Ensure no other process is consuming excessive system resources.", e);
        std::process::exit(1);
    })
});

#[cfg(unix)]
fn setup_signal_handler() {
    use signal_hook::flag;

    let terminate = ui::get_terminate();

    if let Err(e) = flag::register(signal_hook::consts::signal::SIGINT, terminate.clone()) {
        eprintln!("Failed to set Ctrl+C handler: {}", e);
        std::process::exit(1);
    }
    if let Err(e) = flag::register(signal_hook::consts::signal::SIGTERM, terminate) {
        eprintln!("Failed to set SIGTERM handler: {}", e);
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn setup_signal_handler() {
    // Windows: Ctrl+C is handled by crossterm's event loop
}

#[derive(Debug, Parser)]
#[command(
    name = "snp",
    about = "A fast, terminal-based snippet manager with fuzzy search, clipboard support, and cloud sync",
    version = env!("CARGO_PKG_VERSION"),
    after_help = "Config: ~/.config/snp/snippets.toml | Docs: https://github.com/anomalyco/snip-it"
)]
struct Cli {
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
        #[arg(short, long, help = "Sync a specific library")]
        library: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "List connected servers")]
        servers: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Skip conflict prompts (keeps local)")]
        non_interactive: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Upload local changes only")]
        push_only: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Download remote changes only")]
        pull_only: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Show what would be synced")]
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
        #[arg(long, default_value = crate::config::DEFAULT_SERVER_URL)]
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
    /// Generate shell completions
    #[command(alias = "g")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
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
        Commands::Completions { shell } => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(shell, &mut cmd, "snp", &mut std::io::stdout());
        }
    }
    Ok(())
}

fn main() {
    setup_panic_handler();
    setup_signal_handler();
    init_default_logging();
    log_startup_info();

    let cli = Cli::parse();
    if let Err(e) = dispatch_command(cli.command) {
        eprintln!("error: {}", e);
        log_shutdown_info();
        std::process::exit(1);
    }

    log_shutdown_info();
}
