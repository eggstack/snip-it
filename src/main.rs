//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and optional self-hosted sync with end-to-end encryption.

use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use snip_it::CommandOutcome;
use snip_it::auto_sync::SubcommandTag;
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
        /// Include output/notes field in fuzzy search matching
        #[arg(long, action = clap::ArgAction::SetTrue)]
        search_output: bool,
        /// Sort mode for snippet ordering
        #[arg(long, value_enum, default_value_t = snip_it::sort::SnippetSort::Relevance)]
        sort: snip_it::sort::SnippetSort,
        /// Show favorites before other snippets
        #[arg(long, action = clap::ArgAction::SetTrue)]
        favorites_first: bool,
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
        /// Sort mode for snippet ordering
        #[arg(long, value_enum, default_value_t = snip_it::sort::SnippetSort::Relevance)]
        sort: snip_it::sort::SnippetSort,
        /// Show favorites before other snippets
        #[arg(long, action = clap::ArgAction::SetTrue)]
        favorites_first: bool,
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
        /// Sort mode for snippet ordering
        #[arg(long, value_enum, default_value_t = snip_it::sort::SnippetSort::Relevance)]
        sort: snip_it::sort::SnippetSort,
        /// Show favorites before other snippets
        #[arg(long, action = clap::ArgAction::SetTrue)]
        favorites_first: bool,
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
        /// Sort mode for snippet ordering
        #[arg(long, value_enum, default_value_t = snip_it::sort::SnippetSort::Relevance)]
        sort: snip_it::sort::SnippetSort,
        /// Show favorites before other snippets
        #[arg(long, action = clap::ArgAction::SetTrue)]
        favorites_first: bool,
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
        /// Sort mode for snippet ordering
        #[arg(long, value_enum, default_value_t = snip_it::sort::SnippetSort::Relevance)]
        sort: snip_it::sort::SnippetSort,
        /// Show favorites before other snippets
        #[arg(long, action = clap::ArgAction::SetTrue)]
        favorites_first: bool,
    },
    /// Edit the config file in $EDITOR (e)
    #[command(alias = "e")]
    Edit {
        #[arg(short, long)]
        library: Option<String>,
        /// Set the output/notes field on a snippet (requires --filter)
        #[arg(long, conflicts_with_all = ["output_stdin", "clear_output"])]
        output: Option<String>,
        /// Read output/notes field from stdin (requires --filter)
        #[arg(long, conflicts_with_all = ["output", "clear_output"])]
        output_stdin: bool,
        /// Clear the output/notes field (requires --filter)
        #[arg(long, conflicts_with_all = ["output", "output_stdin"])]
        clear_output: bool,
        /// Filter to select which snippet to edit output on (required with output flags)
        #[arg(short, long)]
        filter: Option<String>,
    },
    /// Show keybindings
    #[command(alias = "k")]
    Keybindings,
    /// Sync snippets with server
    #[command(alias = "y")]
    Sync {
        #[command(subcommand)]
        command: Option<SyncCommands>,
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
    /// Diagnose pet file compatibility, installed snp environment, or shell init syntax
    Doctor {
        /// Path to a pet TOML snippet file to analyze
        #[arg(
            long = "pet-file",
            value_name = "PATH",
            conflicts_with_all = ["compatibility", "library", "sync"]
        )]
        pet_file: Option<PathBuf>,
        /// Audit the installed snp environment
        #[arg(long, conflicts_with_all = ["pet_file", "library"])]
        compatibility: bool,
        /// Run focused sync diagnostics using the canonical status snapshot
        #[arg(long, conflicts_with_all = ["pet_file", "library"])]
        sync: bool,
        /// Check shell init output syntax for a specific shell (bash, zsh, fish)
        #[arg(long, value_enum)]
        check_shell: Option<ShellIntegration>,
        /// Check a specific library file for compatibility
        #[arg(
            long,
            value_name = "NAME_OR_PATH",
            conflicts_with_all = ["pet_file", "compatibility", "sync"]
        )]
        library: Option<String>,
        /// Treat warnings as errors
        #[arg(long)]
        strict: bool,
        /// Report output format
        #[arg(long, value_enum, default_value = "human")]
        report: commands::doctor_cmd::DiagnosticReportFormat,
    },
    /// Import snippets from external formats
    #[command(alias = "i")]
    Import {
        #[command(subcommand)]
        command: ImportSubcommands,
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
    /// Show auto-sync status
    Status {
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync_only: bool,
    },
    /// Internal: detached auto-sync worker (hidden, invoked by parent after mutation)
    #[command(name = "auto-sync-worker", hide = true)]
    AutoSyncWorker {
        /// State directory containing pending markers and worker locks
        #[arg(long)]
        state_dir: std::path::PathBuf,
    },
    /// Internal: one-shot sync executor (hidden, invoked by worker)
    #[command(name = "auto-sync-execute", hide = true)]
    AutoSyncExecute {
        /// State directory
        #[arg(long)]
        state_dir: std::path::PathBuf,
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

#[derive(Debug, Subcommand)]
enum SyncCommands {
    /// Run a sync operation (default when no subcommand given)
    #[command(alias = "s")]
    Run {
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
    /// View or update auto-sync policy settings
    #[command(alias = "c")]
    Config {
        /// Show the current effective auto-sync configuration
        #[arg(long, action = clap::ArgAction::SetTrue)]
        show: bool,
        /// Enable or disable auto-sync after mutations
        #[arg(long)]
        auto_sync: Option<String>,
        /// Debounce delay in seconds before auto-sync fires (0-300)
        #[arg(long)]
        debounce: Option<u64>,
        /// Maximum delay in seconds before forcing a sync (0-600)
        #[arg(long)]
        max_delay: Option<u64>,
        /// Failure mode: ignore, warn, or error
        #[arg(long)]
        failure: Option<String>,
        /// Executor sync timeout in seconds (5-120, default 30)
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Retry a failed auto-sync now
    #[command(alias = "r")]
    Retry {
        #[arg(short, long)]
        library: Option<String>,
    },
    /// Clear failure state without discarding pending intent
    #[command(alias = "f")]
    ClearFailure,
    /// Discard pending sync intent
    #[command(alias = "d")]
    DiscardPending {
        #[arg(long, action = clap::ArgAction::SetTrue)]
        force: bool,
        #[arg(long)]
        generation: Option<u64>,
    },
    /// Repair sync control artifacts
    Repair {
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        apply: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ImportSubcommands {
    /// Import a Pet snippet file into a native library
    #[command(alias = "p")]
    Pet {
        /// Path to the Pet TOML snippet file
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Destination library name (derived from filename if omitted)
        #[arg(short, long)]
        library: Option<String>,
        /// Import into an existing library, skipping duplicates
        #[arg(long, conflicts_with = "replace")]
        merge: bool,
        /// Replace the destination library entirely (with backup)
        #[arg(long, conflicts_with = "merge")]
        replace: bool,
        /// Preview changes without writing files
        #[arg(long)]
        dry_run: bool,
        /// Abort on any error-severity diagnostic
        #[arg(long)]
        strict: bool,
        /// Report output format
        #[arg(long, value_enum, default_value = "human")]
        report: commands::import_cmd::ReportFormat,
        /// Write JSON report to a file
        #[arg(long)]
        report_file: Option<PathBuf>,
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
            commands::run_cmd::run(None, false, None, None, &RUNTIME)?;
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
            search_output,
            sort,
            favorites_first,
        }) => {
            let format = if json {
                commands::list_cmd::ListFormat::Json
            } else if csv {
                commands::list_cmd::ListFormat::Csv
            } else {
                commands::list_cmd::ListFormat::Default
            };
            let sort_opts = snip_it::sort::SortOptions {
                mode: sort,
                favorites_first,
            };
            commands::list_cmd::run(
                filter,
                config,
                library,
                format,
                Some(sort_opts),
                search_output,
            )?;
        }
        Some(Commands::Run {
            filter,
            sync,
            library,
            sort,
            favorites_first,
        }) => {
            let sort_opts = snip_it::sort::SortOptions {
                mode: sort,
                favorites_first,
            };
            commands::run_cmd::run(filter, sync, library, Some(sort_opts), &RUNTIME)?;
        }
        Some(Commands::Clip {
            filter,
            sync,
            library,
            sort,
            favorites_first,
        }) => {
            let sort_opts = snip_it::sort::SortOptions {
                mode: sort,
                favorites_first,
            };
            commands::clip_cmd::run(filter, sync, library, None, Some(sort_opts), &RUNTIME)?;
        }
        Some(Commands::Search {
            filter,
            sync,
            library,
            sort,
            favorites_first,
        }) => {
            let sort_opts = snip_it::sort::SortOptions {
                mode: sort,
                favorites_first,
            };
            commands::search_cmd::run(filter, sync, library, None, Some(sort_opts), &RUNTIME)?;
        }
        Some(Commands::Select {
            filter,
            query,
            library,
            raw,
            expanded,
            output_file,
            sort,
            favorites_first,
        }) => {
            let effective_filter = filter.or(query);
            let sort_opts = snip_it::sort::SortOptions {
                mode: sort,
                favorites_first,
            };
            return commands::select_cmd::run(
                effective_filter,
                library,
                raw,
                expanded,
                output_file,
                Some(sort_opts),
                &RUNTIME,
            );
        }
        Some(Commands::Edit {
            library,
            output,
            output_stdin,
            clear_output,
            filter,
        }) => {
            let has_output_flags = output.is_some() || output_stdin || clear_output;
            if has_output_flags {
                let output_value = if clear_output {
                    Some(String::new())
                } else if output_stdin {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).map_err(
                        |e| {
                            snip_it::error::SnipError::io_error(
                                "read stdin",
                                std::path::PathBuf::new(),
                                e,
                            )
                        },
                    )?;
                    Some(buf)
                } else {
                    output
                };
                let filter_str = filter.ok_or_else(|| {
                    snip_it::error::SnipError::runtime_error(
                        "--filter is required when using --output, --output-stdin, or --clear-output",
                        None,
                    )
                })?;
                commands::edit_cmd::run_edit_output(library, filter_str, output_value)?;
            } else {
                commands::edit_cmd::run(library, None)?;
            }
        }
        Some(Commands::Keybindings) => {
            commands::keybindings_cmd::run()?;
        }
        Some(Commands::Sync { command }) => match command {
            None | Some(SyncCommands::Run { .. }) => {
                let (library, servers, push_only, pull_only, dry_run) = match command {
                    Some(SyncCommands::Run {
                        library,
                        servers,
                        push_only,
                        pull_only,
                        dry_run,
                    }) => (library, servers, push_only, pull_only, dry_run),
                    _ => (None, false, false, false, false),
                };
                let options = commands::sync_cmd::SyncOptions {
                    library,
                    servers,
                    push_only,
                    pull_only,
                    dry_run,
                };
                commands::sync_cmd::run(options, &RUNTIME)?;
            }
            Some(SyncCommands::Config {
                show,
                auto_sync,
                debounce,
                max_delay,
                failure,
                timeout,
            }) => {
                commands::sync_cmd::run_config(
                    show, auto_sync, debounce, max_delay, failure, timeout,
                )?;
            }
            Some(SyncCommands::Retry { library }) => {
                commands::sync_cmd::run_retry(library, &RUNTIME)?;
            }
            Some(SyncCommands::ClearFailure) => {
                commands::sync_cmd::run_clear_failure()?;
            }
            Some(SyncCommands::DiscardPending { force, generation }) => {
                commands::sync_cmd::run_discard_pending(force, generation)?;
            }
            Some(SyncCommands::Repair { dry_run, apply }) => {
                commands::sync_cmd::run_repair(dry_run, apply)?;
            }
        },
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
        Some(Commands::Doctor {
            pet_file,
            compatibility,
            sync,
            check_shell,
            library,
            strict,
            report,
        }) => {
            let check_shell_str = check_shell.map(|s| match s {
                ShellIntegration::Bash => "bash".to_string(),
                ShellIntegration::Zsh => "zsh".to_string(),
                ShellIntegration::Fish => "fish".to_string(),
            });
            commands::doctor_cmd::run(
                pet_file,
                compatibility,
                sync,
                check_shell_str,
                library,
                strict,
                report,
            )?;
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
        Some(Commands::Import { command }) => match command {
            ImportSubcommands::Pet {
                path,
                library,
                merge,
                replace,
                dry_run,
                strict,
                report,
                report_file,
            } => {
                let mode = if replace {
                    commands::import_cmd::ImportMode::Replace
                } else if merge {
                    commands::import_cmd::ImportMode::Merge
                } else {
                    commands::import_cmd::ImportMode::Create
                };
                let options = commands::import_cmd::PetImportOptions {
                    source: path,
                    destination_library: library,
                    mode,
                    strict,
                    dry_run,
                    report_format: report,
                    report_file,
                };
                commands::import_cmd::run_import_pet(options)?;
            }
        },
        Some(Commands::Status { json, sync_only }) => {
            commands::status_cmd::run(json, sync_only)?;
        }
        Some(Commands::AutoSyncWorker { state_dir }) => {
            let outcome = snip_it::auto_sync::worker::run(&state_dir);
            match outcome {
                snip_it::auto_sync::WorkerOutcome::Success
                | snip_it::auto_sync::WorkerOutcome::NothingToDo
                | snip_it::auto_sync::WorkerOutcome::Failed => {}
                _ => {}
            }
        }
        Some(Commands::AutoSyncExecute { state_dir }) => {
            let exit_code = snip_it::auto_sync::executor::run_executor(&state_dir);
            std::process::exit(exit_code);
        }
    }
    Ok(CommandOutcome::Success)
}

/// Map a CLI `Commands` variant to a `SubcommandTag` for startup
/// recovery classification.
fn classify_command(cmd: &Commands) -> SubcommandTag {
    match cmd {
        Commands::Sync { .. } => SubcommandTag::Sync,
        Commands::Cron { .. } => SubcommandTag::Cron,
        Commands::Register { .. } => SubcommandTag::Register,
        Commands::AutoSyncWorker { .. } => SubcommandTag::AutoSyncWorker,
        Commands::AutoSyncExecute { .. } => SubcommandTag::AutoSyncExecute,
        Commands::Status { .. } => SubcommandTag::Mutation,
        _ => SubcommandTag::Mutation,
    }
}

fn main() {
    setup_panic_handler();
    setup_signal_handler();
    init_default_logging();
    log_startup_info();

    let cli = Cli::parse();

    if snip_it::auto_sync::should_attempt_auto_sync_recovery(
        cli.command.as_ref().map(classify_command),
    ) {
        snip_it::auto_sync::startup_recover_pending();
    }

    match dispatch_command(cli.command) {
        Ok(CommandOutcome::Success) => {}
        Ok(CommandOutcome::Cancelled) => {
            log_shutdown_info();
            std::process::exit(4);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {e}");
            log_shutdown_info();
            std::process::exit(1);
        }
    }

    log_shutdown_info();
}
