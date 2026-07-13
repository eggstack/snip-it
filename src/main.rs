//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and optional self-hosted sync with end-to-end encryption.

use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use snip_it::CommandOutcome;
use snip_it::commands;
use snip_it::config;
use snip_it::error::SnipResult;
use snip_it::logging::{
    init_default_logging, log_shutdown_info, log_startup_info, setup_panic_handler,
};

mod update;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("Failed to create async runtime: {e}. Ensure no other process is consuming excessive system resources.");
        std::process::exit(1);
    })
});

#[cfg(unix)]
fn setup_signal_handler() {
    use signal_hook::flag;

    use snip_it::ui;

    let terminate = ui::get_terminate();

    if let Err(e) = flag::register(signal_hook::consts::signal::SIGINT, terminate.clone()) {
        eprintln!("Failed to set Ctrl+C handler: {e}");
        std::process::exit(1);
    }
    if let Err(e) = flag::register(signal_hook::consts::signal::SIGTERM, terminate) {
        eprintln!("Failed to set SIGTERM handler: {e}");
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
    about = "A fast, terminal-based snippet manager with fuzzy search, clipboard support, and optional self-hosted sync",
    version = env!("CARGO_PKG_VERSION"),
    after_help = "Config: ~/.config/snp/snippets.toml | Docs: https://github.com/eggstack/snip-it"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show version (v)
    #[command(alias = "v")]
    Version,
    /// Check for and install an update using the current installation method
    Update {
        #[arg(long, help = "Check for an update without installing it")]
        dry_run: bool,
        #[arg(long, help = "Use Cargo's locked dependency versions")]
        locked: bool,
    },
    /// Create a new snippet (n)
    #[command(alias = "n")]
    New {
        /// Command text supplied as a positional argument.
        #[arg(
            value_name = "COMMAND",
            conflicts_with_all = ["command_stdin", "multiline", "from_file", "editor"]
        )]
        command: Option<String>,
        /// Prompt for tags, or provide comma/space-separated tags directly.
        #[arg(
            short,
            long,
            action = clap::ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "__snp_prompt_tags__",
            value_name = "TAGS"
        )]
        tags: Option<String>,
        #[arg(
            short,
            long,
            action = clap::ArgAction::SetTrue,
            conflicts_with_all = ["command_stdin", "editor"]
        )]
        multiline: bool,
        /// Read the command body byte-for-byte from stdin.
        #[arg(
            long,
            action = clap::ArgAction::SetTrue,
            conflicts_with_all = ["command", "multiline", "from_file", "editor"]
        )]
        command_stdin: bool,
        /// Read command body from a file.
        #[arg(
            long = "from-file",
            value_name = "PATH",
            conflicts_with_all = ["command", "command_stdin", "editor"]
        )]
        from_file: Option<PathBuf>,
        /// Open $VISUAL (or $EDITOR) to write the command body.
        #[arg(
            long,
            action = clap::ArgAction::SetTrue,
            conflicts_with_all = ["command", "command_stdin", "from_file"]
        )]
        editor: bool,
        #[arg(short = 'd', long)]
        description: Option<String>,
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
        #[arg(conflicts_with = "csv")]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        #[arg(conflicts_with = "json")]
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
    /// Select a snippet and print its command to stdout (no execution)
    #[command(alias = "sel")]
    Select {
        #[arg(short, long)]
        filter: Option<String>,
        /// Initial query to pre-fill the search (alias for --filter)
        #[arg(long)]
        query: Option<String>,
        #[arg(short, long)]
        library: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "expanded")]
        raw: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "raw")]
        expanded: bool,
        /// Write selection to file instead of stdout (used by shell integration)
        #[arg(long)]
        output_file: Option<PathBuf>,
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
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Upload local changes only")]
        #[arg(conflicts_with = "pull_only")]
        push_only: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Download remote changes only")]
        #[arg(conflicts_with = "push_only")]
        pull_only: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, help = "Show what would be synced")]
        dry_run: bool,
    },
    /// Setup automatic sync with cron
    #[command(alias = "cr")]
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
    /// Generate interactive shell integration (functions/widgets)
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
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
    /// Search premade libraries by query
    #[command(alias = "se")]
    Search { query: String },
    /// Update a premade library (show diff and re-download)
    #[command(alias = "u")]
    Update { name: String },
}

#[derive(Debug, Subcommand)]
enum ShellCommands {
    /// Generate shell integration code for the specified shell
    #[command(alias = "i")]
    Init {
        /// Shell to generate integration for
        #[arg(value_enum)]
        shell: ShellIntegration,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ShellIntegration {
    /// Bash shell integration
    Bash,
    /// Zsh shell integration
    Zsh,
    /// Fish shell integration
    Fish,
}

fn dispatch_command(cli: Option<Commands>) -> SnipResult<CommandOutcome> {
    match cli {
        None => {
            commands::run_cmd::run(None, false, None, &RUNTIME)?;
        }
        Some(Commands::Version) => {
            println!("snp {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Update { dry_run, locked }) => {
            update::run(dry_run, locked).map_err(|error| {
                snip_it::error::SnipError::runtime_error("update failed", Some(&error))
            })?;
        }
        Some(Commands::New {
            command,
            tags,
            multiline,
            command_stdin,
            from_file,
            editor,
            description,
            config,
            library,
        }) => {
            commands::new_cmd::run(
                command,
                description,
                tags,
                multiline,
                command_stdin,
                from_file,
                editor,
                config,
                library,
            )?;
        }
        Some(Commands::List {
            filter,
            config,
            library,
            json,
            csv,
        }) => {
            let format = if json {
                commands::list_cmd::ListFormat::Json
            } else if csv {
                commands::list_cmd::ListFormat::Csv
            } else {
                commands::list_cmd::ListFormat::Default
            };
            commands::list_cmd::run(filter, config, library, format)?;
        }
        Some(Commands::Run {
            filter,
            sync,
            library,
        }) => {
            commands::run_cmd::run(filter, sync, library, &RUNTIME)?;
        }
        Some(Commands::Clip {
            filter,
            sync,
            library,
        }) => {
            commands::clip_cmd::run(filter, sync, library, None, &RUNTIME)?;
        }
        Some(Commands::Search {
            filter,
            sync,
            library,
        }) => {
            commands::search_cmd::run(filter, sync, library, None, &RUNTIME)?;
        }
        Some(Commands::Select {
            filter,
            query,
            library,
            raw,
            expanded,
            output_file,
        }) => {
            let effective_filter = filter.or(query);
            return commands::select_cmd::run(
                effective_filter,
                library,
                raw,
                expanded,
                output_file,
                &RUNTIME,
            );
        }
        Some(Commands::Edit { library }) => {
            commands::edit_cmd::run(library, None)?;
        }
        Some(Commands::Keybindings) => {
            commands::keybindings_cmd::run()?;
        }
        Some(Commands::Sync {
            library,
            servers,
            push_only,
            pull_only,
            dry_run,
        }) => {
            let options = commands::sync_cmd::SyncOptions {
                library,
                servers,
                push_only,
                pull_only,
                dry_run,
            };
            commands::sync_cmd::run(options, &RUNTIME)?;
        }
        Some(Commands::Cron { interval }) => {
            commands::cron_cmd::run(interval)?;
        }
        Some(Commands::Register { server, force }) => {
            commands::register_cmd::run(server, force, &RUNTIME)?;
        }
        Some(Commands::Library { command }) => match command {
            LibraryCommands::List => commands::library_cmd::run_list()?,
            LibraryCommands::Create { name } => commands::library_cmd::run_create(name)?,
            LibraryCommands::Delete { name, force } => {
                commands::library_cmd::run_delete(name, force)?
            }
            LibraryCommands::SetPrimary { name } => commands::library_cmd::run_set_primary(name)?,
            LibraryCommands::Show { name } => commands::library_cmd::run_show(name)?,
        },
        Some(Commands::Premade { command }) => match command {
            PremadeCommands::List => commands::premade_cmd::run_list(&RUNTIME)?,
            PremadeCommands::Get { name } => {
                let all = name.as_ref().is_some_and(|n| n == "all");
                commands::premade_cmd::run_get(name, all, &RUNTIME)?;
            }
            PremadeCommands::Sync => commands::premade_cmd::run_sync(&RUNTIME)?,
            PremadeCommands::Search { query } => {
                commands::premade_cmd::run_search(query, &RUNTIME)?;
            }
            PremadeCommands::Update { name } => {
                commands::premade_cmd::run_update(name, &RUNTIME)?;
            }
        },
        Some(Commands::Completions { shell }) => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(shell, &mut cmd, "snp", &mut std::io::stdout());
        }
        Some(Commands::Shell { command }) => match command {
            ShellCommands::Init { shell } => {
                let shell_type = match shell {
                    ShellIntegration::Bash => commands::shell_cmd::ShellType::Bash,
                    ShellIntegration::Zsh => commands::shell_cmd::ShellType::Zsh,
                    ShellIntegration::Fish => commands::shell_cmd::ShellType::Fish,
                };
                commands::shell_cmd::run(shell_type)?;
            }
        },
    }
    Ok(CommandOutcome::Success)
}

fn main() {
    setup_panic_handler();
    setup_signal_handler();
    init_default_logging();
    log_startup_info();

    let cli = Cli::parse();
    match dispatch_command(cli.command) {
        Ok(CommandOutcome::Success) => {}
        Ok(CommandOutcome::Cancelled) => {
            log_shutdown_info();
            std::process::exit(4);
        }
        Err(e) => {
            eprintln!("error: {e}");
            log_shutdown_info();
            std::process::exit(1);
        }
    }

    log_shutdown_info();
}
