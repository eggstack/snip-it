//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and optional self-hosted sync with end-to-end encryption.

use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use snip_it::auto_sync::StartupRecoveryPolicy;
use snip_it::commands;
use snip_it::error::SnipResult;
use snip_it::logging::{
    init_default_file_logging, log_shutdown_info, log_startup_info, setup_panic_handler,
};
use snip_it::outcome::CliOutcome;

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
    after_help = "Exit codes:\n  0  success\n  1  general error\n  2  usage/argument error\n  3  not found\n  4  cancelled\n  5  ambiguous match\n  6  validation failure\n  7  sync failure\n  8  execution failure\n  9  conflict/refused\n\nConfig: ~/.config/snp/snippets.toml\nDocs: https://github.com/eggstack/snip-it\nShell: snp shell init bash|zsh|fish"
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
    /// Internal Windows self-replacement helper.
    #[command(name = "__self-replace", hide = true)]
    SelfReplace {
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        destination: PathBuf,
    },
    /// Create a new snippet (n)
    #[command(alias = "n")]
    New(commands::new_cmd::NewArgs),
    /// List all snippets (l) — never executes
    #[command(alias = "l")]
    List(commands::list_cmd::ListArgs),
    /// Run a snippet via TUI selection (r) — executes via shell
    #[command(alias = "r")]
    Run(commands::run_cmd::RunArgs),
    /// Copy a snippet to clipboard via TUI selection (c)
    #[command(alias = "c")]
    Clip(commands::clip_cmd::ClipArgs),
    /// Search for a snippet via TUI selection (s)
    #[command(alias = "s")]
    Search(commands::search_cmd::SearchArgs),
    /// Select a snippet and print its command to stdout (no execution)
    #[command(alias = "sel")]
    Select(commands::select_cmd::SelectArgs),
    /// Edit the config file in $EDITOR (e)
    #[command(alias = "e")]
    Edit(commands::edit_cmd::EditArgs),
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
    Cron(commands::cron_cmd::CronArgs),
    /// Register a new sync account
    #[command(alias = "reg")]
    Register(commands::register_cmd::RegisterArgs),
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
    Doctor(commands::doctor_cmd::DoctorArgs),
    /// Import snippets from external formats
    #[command(alias = "i")]
    Import {
        #[command(subcommand)]
        command: ImportSubcommands,
    },
    /// Repair configuration and library files
    #[command(alias = "rp")]
    Repair(commands::repair_cmd::RepairArgs),
    /// Validate snippet data (read-only)
    #[command(alias = "val")]
    Validate(commands::validate_cmd::ValidateArgs),
    /// Advanced data maintenance commands
    #[command(alias = "d")]
    Data {
        #[command(subcommand)]
        command: DataCommands,
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
    /// Create a secret-free backup snapshot
    Backup(commands::backup_cmd::BackupArgs),
    /// Restore from a backup snapshot
    Restore(commands::restore_cmd::RestoreArgs),
    /// Show auto-sync status (read-only)
    Status(commands::status_cmd::StatusArgs),
    /// Expose snippets to local coding agents over MCP
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    /// Retrieve a snippet deterministically (no TUI, no execution)
    Get(commands::get_cmd::GetArgs),
    /// Internal: detached auto-sync worker (hidden, invoked by parent after mutation)
    #[command(name = "auto-sync-worker", hide = true)]
    AutoSyncWorker {
        /// State directory containing pending markers and worker locks
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
        shell: commands::shell_cmd::ShellIntegration,
    },
}

#[derive(Debug, Subcommand)]
enum McpCommands {
    /// Run the read-only newline-delimited stdio MCP server
    Serve,
    /// Print setup instructions for a supported MCP client
    Instructions {
        #[arg(value_enum)]
        client: snip_it::mcp::McpClient,
    },
    /// Register snip-it with a supported MCP client when its official CLI is safe to use
    Install {
        #[arg(value_enum)]
        client: snip_it::mcp::McpClient,
    },
}

#[derive(Debug, Subcommand)]
enum DataCommands {
    /// Validate snippet data (read-only)
    #[command(alias = "v")]
    Validate(commands::validate_cmd::ValidateArgs),
    /// Create a secret-free backup snapshot
    #[command(alias = "b")]
    Backup(commands::backup_cmd::BackupArgs),
    /// Restore from a backup snapshot
    Restore(commands::restore_cmd::RestoreArgs),
    /// Repair configuration and library files
    Repair(commands::repair_cmd::RepairArgs),
    /// Show auto-sync status (read-only)
    #[command(alias = "s")]
    Status(commands::status_cmd::StatusArgs),
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

/// Map a `RepairExitStatus` to the appropriate process exit code.
/// Clean/DryRun/Repaired → 0 (implicit), UnsafeOnly → 10, PartialFailure → 1.
///
/// `UnsafeOnly` deliberately does not reuse 2 (`USAGE_ERROR`): a valid
/// invocation that found unsafe repairs awaiting an operator decision is
/// not a usage error, and scripts must be able to tell the two apart.
fn exit_on_repair_status(status: commands::repair_cmd::RepairExitStatus) {
    match status {
        commands::repair_cmd::RepairExitStatus::Clean
        | commands::repair_cmd::RepairExitStatus::Repaired
        | commands::repair_cmd::RepairExitStatus::DryRun => {}
        commands::repair_cmd::RepairExitStatus::PartialFailure => {
            std::process::exit(snip_it::outcome::exit_code::GENERAL_ERROR);
        }
        commands::repair_cmd::RepairExitStatus::UnsafeOnly => {
            std::process::exit(snip_it::outcome::exit_code::UNSAFE_REPAIRS);
        }
    }
}

fn report_ambiguous(
    identities: &[snip_it::selector::SnippetIdentity],
) -> snip_it::outcome::CliOutcome {
    for identity in identities {
        eprintln!(
            "  {} - {} ({})",
            identity.id, identity.description, identity.library_name
        );
    }
    snip_it::outcome::CliOutcome::Ambiguous
}

/// Single-path handler for `validate` (top-level and `data` spellings).
///
/// Both spellings share `ValidateArgs`, so this is the only validation,
/// JSON formatting, and exit-code mapping for the operation.
fn handle_validate(args: commands::validate_cmd::ValidateArgs) -> SnipResult<CliOutcome> {
    commands::validate_cmd::run(args.library, args.strict, args.json)
}

/// Single-path handler for `backup` (top-level and `data` spellings).
fn handle_backup(args: commands::backup_cmd::BackupArgs) -> SnipResult<CliOutcome> {
    commands::backup_cmd::run(
        args.output,
        args.include_usage,
        args.include_sync_state,
        args.format,
        args.json,
    )?;
    Ok(CliOutcome::Success)
}

/// Single-path handler for `restore` (top-level and `data` spellings).
fn handle_restore(args: commands::restore_cmd::RestoreArgs) -> SnipResult<CliOutcome> {
    commands::restore_cmd::run(args.backup, args.mode, args.json)?;
    Ok(CliOutcome::Success)
}

/// Single-path handler for `repair` (top-level and `data` spellings).
///
/// `UnsafeOnly` (exit 10) and `PartialFailure` (exit 1) exit directly via
/// `exit_on_repair_status`; clean/dry-run/repaired map to `Success`.
fn handle_repair(args: commands::repair_cmd::RepairArgs) -> SnipResult<CliOutcome> {
    let status = commands::repair_cmd::run(args.dry_run, args.apply, args.library, args.json)?;
    exit_on_repair_status(status);
    Ok(CliOutcome::Success)
}

/// Single-path handler for `status` (top-level and `data` spellings).
fn handle_status(args: commands::status_cmd::StatusArgs) -> SnipResult<CliOutcome> {
    commands::status_cmd::run(args.json, args.sync_only)?;
    Ok(CliOutcome::Success)
}

fn dispatch_command(cli: Option<Commands>) -> SnipResult<CliOutcome> {
    match cli {
        None => {
            return commands::run_cmd::run(None, false, None, None, None);
        }
        Some(Commands::Version) => {
            println!("snp {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Update { dry_run, locked }) => {
            update::run(dry_run, locked).map_err(|error| {
                snip_it::error::SnipError::runtime_error("update failed", Some(&error))
            })?;
        }
        Some(Commands::SelfReplace {
            candidate,
            destination,
        }) => {
            update::run_self_replace_helper(&candidate, &destination).map_err(|error| {
                snip_it::error::SnipError::runtime_error("self-replacement failed", Some(&error))
            })?;
        }
        Some(Commands::New(args)) => {
            commands::new_cmd::run(
                args.command,
                args.description,
                args.tags,
                args.multiline,
                args.command_stdin,
                args.from_file,
                args.editor,
                args.config,
                args.library,
            )?;
        }
        Some(Commands::List(args)) => {
            let format = if args.json {
                commands::list_cmd::ListFormat::Json
            } else if args.csv {
                commands::list_cmd::ListFormat::Csv
            } else {
                commands::list_cmd::ListFormat::Default
            };
            let sort_opts = snip_it::sort::SortOptions {
                mode: args.sort,
                favorites_first: args.favorites_first,
            };
            commands::list_cmd::run(
                args.filter,
                args.config,
                args.library,
                format,
                Some(sort_opts),
                args.search_output,
            )?;
        }
        Some(Commands::Run(args)) => {
            if args.id.is_some() || args.description_exact.is_some() || args.command_exact.is_some()
            {
                let result = snip_it::selector::resolve_exact_target(
                    args.library,
                    args.id,
                    args.description_exact,
                    args.command_exact,
                )?;
                let outcome = match result {
                    snip_it::selector::SelectionResult::One(m) => {
                        let outcome = commands::run_cmd::run_exact(
                            &m.snippet,
                            args.sync,
                            args.sync.then_some(&RUNTIME),
                        )?;
                        match outcome {
                            CliOutcome::ExecutionFailed { child_code } => {
                                std::process::exit(child_code.unwrap_or(8));
                            }
                            _ => snip_it::outcome::CliOutcome::Success,
                        }
                    }
                    snip_it::selector::SelectionResult::Ambiguous(identities) => {
                        report_ambiguous(&identities)
                    }
                    _ => snip_it::outcome::CliOutcome::NotFound,
                };
                return Ok(outcome);
            } else {
                let sort_opts = snip_it::sort::SortOptions {
                    mode: args.sort,
                    favorites_first: args.favorites_first,
                };
                let outcome = commands::run_cmd::run(
                    args.filter,
                    args.sync,
                    args.library,
                    Some(sort_opts),
                    args.sync.then_some(&RUNTIME),
                )?;
                if let CliOutcome::ExecutionFailed { child_code } = outcome {
                    std::process::exit(child_code.unwrap_or(8));
                }
            }
        }
        Some(Commands::Clip(args)) => {
            if args.id.is_some() || args.description_exact.is_some() || args.command_exact.is_some()
            {
                let result = snip_it::selector::resolve_exact_target(
                    args.library,
                    args.id,
                    args.description_exact,
                    args.command_exact,
                )?;
                let outcome = match result {
                    snip_it::selector::SelectionResult::One(m) => {
                        commands::clip_cmd::run_exact(
                            &m.snippet,
                            args.sync,
                            args.sync.then_some(&RUNTIME),
                        )?;
                        snip_it::outcome::CliOutcome::Success
                    }
                    snip_it::selector::SelectionResult::Ambiguous(identities) => {
                        report_ambiguous(&identities)
                    }
                    _ => snip_it::outcome::CliOutcome::NotFound,
                };
                return Ok(outcome);
            } else {
                let sort_opts = snip_it::sort::SortOptions {
                    mode: args.sort,
                    favorites_first: args.favorites_first,
                };
                commands::clip_cmd::run(
                    args.filter,
                    args.sync,
                    args.library,
                    None,
                    Some(sort_opts),
                    args.sync.then_some(&RUNTIME),
                )?;
            }
        }
        Some(Commands::Search(args)) => {
            let sort_opts = snip_it::sort::SortOptions {
                mode: args.sort,
                favorites_first: args.favorites_first,
            };
            commands::search_cmd::run(
                args.filter,
                args.sync,
                args.library,
                None,
                Some(sort_opts),
                args.sync.then_some(&RUNTIME),
            )?;
        }
        Some(Commands::Select(args)) => {
            let effective_filter = args.filter.or(args.query);
            let sort_opts = snip_it::sort::SortOptions {
                mode: args.sort,
                favorites_first: args.favorites_first,
            };
            return commands::select_cmd::run(
                effective_filter,
                args.library,
                args.raw,
                args.expanded,
                args.output_file,
                Some(sort_opts),
            );
        }
        Some(Commands::Edit(args)) => {
            let has_output_flags = args.output.is_some() || args.output_stdin || args.clear_output;
            let has_exact = args.id.is_some()
                || args.description_exact.is_some()
                || args.command_exact.is_some();
            if has_output_flags {
                let output_value = if args.clear_output {
                    Some(String::new())
                } else if args.output_stdin {
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
                    args.output
                };
                if has_exact {
                    let lib_for_edit = args.library.clone();
                    let result = snip_it::selector::resolve_exact_target(
                        args.library,
                        args.id,
                        args.description_exact,
                        args.command_exact,
                    )?;
                    let outcome = match result {
                        snip_it::selector::SelectionResult::One(m) => {
                            commands::edit_cmd::run_edit_output_by_id(
                                lib_for_edit,
                                &m.snippet.id,
                                output_value,
                            )?;
                            snip_it::outcome::CliOutcome::Success
                        }
                        snip_it::selector::SelectionResult::Ambiguous(identities) => {
                            report_ambiguous(&identities)
                        }
                        _ => snip_it::outcome::CliOutcome::NotFound,
                    };
                    return Ok(outcome);
                } else {
                    let filter_str = args.filter.ok_or_else(|| {
                        snip_it::error::SnipError::runtime_error(
                            "--filter is required when using --output, --output-stdin, or --clear-output",
                            None,
                        )
                    })?;
                    commands::edit_cmd::run_edit_output(args.library, filter_str, output_value)?;
                }
            } else {
                commands::edit_cmd::run(args.library, None)?;
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
                let outcome = commands::sync_cmd::run_discard_pending(force, generation)?;
                return Ok(outcome);
            }
            Some(SyncCommands::Repair { dry_run, apply }) => {
                commands::sync_cmd::run_repair(dry_run, apply)?;
            }
        },
        Some(Commands::Cron(args)) => {
            commands::cron_cmd::run(args.interval)?;
        }
        Some(Commands::Register(args)) => {
            commands::register_cmd::run(args.server, args.force, &RUNTIME)?;
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
        Some(Commands::Doctor(args)) => {
            let check_shell_str = args.check_shell.map(|s| s.as_str().to_string());
            let outcome = commands::doctor_cmd::run(
                args.pet_file,
                args.compatibility,
                args.sync,
                check_shell_str,
                args.library,
                args.strict,
                args.report,
            )?;
            return Ok(outcome);
        }
        Some(Commands::Shell { command }) => match command {
            ShellCommands::Init { shell } => {
                commands::shell_cmd::run(shell.to_shell_type())?;
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
        Some(Commands::Repair(args)) => {
            return handle_repair(args);
        }
        Some(Commands::Validate(args)) => {
            return handle_validate(args);
        }
        Some(Commands::Backup(args)) => {
            return handle_backup(args);
        }
        Some(Commands::Restore(args)) => {
            return handle_restore(args);
        }
        Some(Commands::Status(args)) => {
            return handle_status(args);
        }
        Some(Commands::Mcp { command }) => match command {
            McpCommands::Serve => snip_it::mcp::serve()?,
            McpCommands::Instructions { client } => snip_it::mcp::instructions(client)?,
            McpCommands::Install { client } => snip_it::mcp::install(client)?,
        },
        Some(Commands::Data { command }) => match command {
            DataCommands::Validate(args) => {
                return handle_validate(args);
            }
            DataCommands::Backup(args) => {
                return handle_backup(args);
            }
            DataCommands::Restore(args) => {
                return handle_restore(args);
            }
            DataCommands::Repair(args) => {
                return handle_repair(args);
            }
            DataCommands::Status(args) => {
                return handle_status(args);
            }
        },
        Some(Commands::Get(args)) => {
            let outcome = commands::get_cmd::run(
                args.id,
                args.description_exact,
                args.command_exact,
                args.query,
                args.library,
                args.field,
                args.raw,
                args.expanded,
                args.json,
                args.resolution,
                args.vars,
            )?;
            return Ok(outcome);
        }
        Some(Commands::AutoSyncWorker { state_dir }) => {
            let outcome = snip_it::auto_sync::worker::run(&state_dir);
            match outcome {
                snip_it::auto_sync::WorkerOutcome::Success
                | snip_it::auto_sync::WorkerOutcome::NothingToDo => {}
                snip_it::auto_sync::WorkerOutcome::Failed => {
                    std::process::exit(snip_it::outcome::exit_code::GENERAL_ERROR)
                }
                _ => std::process::exit(snip_it::outcome::exit_code::GENERAL_ERROR),
            }
        }
    }
    Ok(CliOutcome::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupServices {
    Minimal,
    Logging,
}

/// Combined command behavior classification. One match over the CLI enum
/// assigns both the startup recovery policy and the logging/audit service
/// level, preventing drift between the two.
struct CommandBehavior {
    recovery: StartupRecoveryPolicy,
    services: StartupServices,
}

fn command_behavior(cmd: Option<&Commands>) -> CommandBehavior {
    let (recovery, services) = match cmd {
        // ── Read-only commands ──────────────────────────────────────
        Some(
            Commands::Version
            | Commands::List(_)
            | Commands::Select(_)
            | Commands::Status(_)
            | Commands::Mcp { .. }
            | Commands::Get(_)
            | Commands::Validate(_)
            | Commands::Backup(_)
            | Commands::Library {
                command: LibraryCommands::List | LibraryCommands::Show { .. },
            },
        ) => (
            StartupRecoveryPolicy::SuppressReadOnly,
            StartupServices::Minimal,
        ),

        // ── Dry-run / read-only modes of otherwise-mutating commands ─
        Some(Commands::Restore(args))
            if args.mode == commands::restore_cmd::RestoreMode::DryRun =>
        {
            (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            )
        }
        Some(Commands::Repair(args)) if args.dry_run => (
            StartupRecoveryPolicy::SuppressReadOnly,
            StartupServices::Minimal,
        ),
        Some(Commands::Import {
            command: ImportSubcommands::Pet { dry_run: true, .. },
        }) => (
            StartupRecoveryPolicy::SuppressReadOnly,
            StartupServices::Minimal,
        ),

        // ── Data subcommand group ───────────────────────────────────
        Some(Commands::Data { command }) => match command {
            DataCommands::Validate(_) | DataCommands::Status(_) | DataCommands::Backup(_) => (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            ),
            DataCommands::Restore(args)
                if args.mode == commands::restore_cmd::RestoreMode::DryRun =>
            {
                (
                    StartupRecoveryPolicy::SuppressReadOnly,
                    StartupServices::Minimal,
                )
            }
            DataCommands::Repair(args) if args.dry_run => (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            ),
            DataCommands::Repair(_) | DataCommands::Restore(_) => {
                (StartupRecoveryPolicy::Allow, StartupServices::Logging)
            }
        },

        // ── Mutation commands: allow recovery, full logging+audit ───
        Some(
            Commands::New(_)
            | Commands::Run(_)
            | Commands::Clip(_)
            | Commands::Search(_)
            | Commands::Edit(_)
            | Commands::Import { .. }
            | Commands::Repair(_)
            | Commands::Restore(_)
            | Commands::Premade { .. }
            | Commands::Library {
                command:
                    LibraryCommands::Create { .. }
                    | LibraryCommands::Delete { .. }
                    | LibraryCommands::SetPrimary { .. },
            },
        ) => (StartupRecoveryPolicy::Allow, StartupServices::Logging),

        // ── Explicit sync commands: suppress recovery, logging only ──
        Some(Commands::Sync { .. } | Commands::Cron(_) | Commands::Register(_)) => (
            StartupRecoveryPolicy::SuppressExplicitSync,
            StartupServices::Logging,
        ),

        // ── Internal worker subprocess ──────────────────────────────
        Some(Commands::AutoSyncWorker { .. }) => (
            StartupRecoveryPolicy::SuppressInternal,
            StartupServices::Logging,
        ),

        // ── Config/setup commands ───────────────────────────────────
        Some(
            Commands::Update { .. }
            | Commands::SelfReplace { .. }
            | Commands::Doctor(_)
            | Commands::Completions { .. }
            | Commands::Shell { .. }
            | Commands::Keybindings,
        ) => (
            StartupRecoveryPolicy::SuppressConfiguration,
            StartupServices::Logging,
        ),

        // ── No subcommand (default TUI) ─────────────────────────────
        None => (StartupRecoveryPolicy::Allow, StartupServices::Logging),
    };
    CommandBehavior { recovery, services }
}

fn main() {
    setup_panic_handler();
    setup_signal_handler();

    let cli = Cli::parse();
    let behavior = command_behavior(cli.command.as_ref());
    match behavior.services {
        StartupServices::Minimal => {}
        StartupServices::Logging => init_default_file_logging(),
    }
    if behavior.services != StartupServices::Minimal {
        log_startup_info();
    }

    if snip_it::auto_sync::should_attempt_auto_sync_recovery_for_policy(Some(behavior.recovery)) {
        snip_it::auto_sync::notification::startup_recover_pending();
    }

    match dispatch_command(cli.command) {
        Ok(CliOutcome::Success) => {}
        Ok(outcome) => {
            if behavior.services != StartupServices::Minimal {
                log_shutdown_info();
            }
            std::process::exit(outcome.exit_code());
        }
        Err(e) => {
            eprintln!("error: {e}");
            if behavior.services != StartupServices::Minimal {
                log_shutdown_info();
            }
            std::process::exit(1);
        }
    }

    if behavior.services != StartupServices::Minimal {
        log_shutdown_info();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn behavior(cmd: Option<&Commands>) -> CommandBehavior {
        command_behavior(cmd)
    }

    #[test]
    fn cli_schema_is_valid() {
        <Cli as clap::CommandFactory>::command().debug_assert();
    }

    // ── Read-only commands ──────────────────────────────────────────

    #[test]
    fn version_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Version));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn list_is_minimal_readonly() {
        let b = behavior(Some(&Commands::List(commands::list_cmd::ListArgs {
            filter: None,
            config: None,
            library: None,
            json: false,
            csv: false,
            search_output: false,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn search_retains_mutation_capabilities() {
        let b = behavior(Some(&Commands::Search(commands::search_cmd::SearchArgs {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn select_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Select(commands::select_cmd::SelectArgs {
            filter: None,
            query: None,
            library: None,
            raw: false,
            expanded: false,
            output_file: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn status_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Status(commands::status_cmd::StatusArgs {
            json: false,
            sync_only: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn get_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Get(commands::get_cmd::GetArgs {
            id: None,
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            field: None,
            raw: false,
            expanded: false,
            json: false,
            resolution: snip_it::selector::ResolutionPolicy::Unique,
            vars: None,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn validate_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Validate(
            commands::validate_cmd::ValidateArgs {
                library: None,
                strict: false,
                json: false,
            },
        )));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn backup_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Backup(commands::backup_cmd::BackupArgs {
            output: None,
            include_usage: false,
            include_sync_state: false,
            format: commands::backup_cmd::BackupFormat::Directory,
            json: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn library_list_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::List,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn library_show_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::Show { name: None },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    // ── Dry-run / read-only modes ───────────────────────────────────

    #[test]
    fn restore_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Restore(
            commands::restore_cmd::RestoreArgs {
                backup: PathBuf::from("/tmp/backup"),
                mode: commands::restore_cmd::RestoreMode::DryRun,
                json: false,
            },
        )));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn import_pet_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Import {
            command: ImportSubcommands::Pet {
                path: PathBuf::from("/tmp/pet.toml"),
                library: None,
                merge: false,
                replace: false,
                dry_run: true,
                strict: false,
                report: commands::import_cmd::ReportFormat::Human,
                report_file: None,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn repair_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Repair(commands::repair_cmd::RepairArgs {
            dry_run: true,
            apply: false,
            library: None,
            json: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    // ── Data subcommand group ───────────────────────────────────────

    #[test]
    fn data_validate_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Validate(commands::validate_cmd::ValidateArgs {
                library: None,
                strict: false,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_status_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Status(commands::status_cmd::StatusArgs {
                json: false,
                sync_only: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_backup_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Backup(commands::backup_cmd::BackupArgs {
                output: None,
                include_usage: false,
                include_sync_state: false,
                format: commands::backup_cmd::BackupFormat::Directory,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_restore_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Restore(commands::restore_cmd::RestoreArgs {
                backup: PathBuf::from("/tmp/b"),
                mode: commands::restore_cmd::RestoreMode::DryRun,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_repair_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Repair(commands::repair_cmd::RepairArgs {
                dry_run: true,
                apply: false,
                library: None,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_repair_mutation_is_allowed() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Repair(commands::repair_cmd::RepairArgs {
                dry_run: false,
                apply: true,
                library: None,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn data_restore_mutation_is_allowed() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Restore(commands::restore_cmd::RestoreArgs {
                backup: PathBuf::from("/tmp/b"),
                mode: commands::restore_cmd::RestoreMode::Merge,
                json: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Mutation commands ───────────────────────────────────────────

    #[test]
    fn new_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::New(commands::new_cmd::NewArgs {
            command: None,
            tags: None,
            multiline: false,
            command_stdin: false,
            from_file: None,
            editor: false,
            description: None,
            config: None,
            library: None,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn run_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Run(commands::run_cmd::RunArgs {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
            id: None,
            description_exact: None,
            command_exact: None,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn clip_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Clip(commands::clip_cmd::ClipArgs {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
            id: None,
            description_exact: None,
            command_exact: None,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn edit_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Edit(commands::edit_cmd::EditArgs {
            library: None,
            output: None,
            output_stdin: false,
            clear_output: false,
            filter: None,
            id: None,
            description_exact: None,
            command_exact: None,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn import_mutation_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Import {
            command: ImportSubcommands::Pet {
                path: PathBuf::from("/tmp/pet.toml"),
                library: None,
                merge: false,
                replace: false,
                dry_run: false,
                strict: false,
                report: commands::import_cmd::ReportFormat::Human,
                report_file: None,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn repair_mutation_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Repair(commands::repair_cmd::RepairArgs {
            dry_run: false,
            apply: true,
            library: None,
            json: false,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn restore_mutation_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Restore(
            commands::restore_cmd::RestoreArgs {
                backup: PathBuf::from("/tmp/b"),
                mode: commands::restore_cmd::RestoreMode::Merge,
                json: false,
            },
        )));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn premade_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Premade {
            command: PremadeCommands::List,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn library_create_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::Create {
                name: "test".to_string(),
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn library_delete_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::Delete {
                name: "test".to_string(),
                force: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn library_set_primary_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::SetPrimary {
                name: "test".to_string(),
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Explicit sync commands ──────────────────────────────────────

    #[test]
    fn sync_is_suppressed_explicit_logging() {
        let b = behavior(Some(&Commands::Sync { command: None }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressExplicitSync);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn sync_run_is_suppressed_explicit_logging() {
        let b = behavior(Some(&Commands::Sync {
            command: Some(SyncCommands::Run {
                library: None,
                servers: false,
                push_only: false,
                pull_only: false,
                dry_run: false,
            }),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressExplicitSync);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn cron_is_suppressed_explicit_logging() {
        let b = behavior(Some(&Commands::Cron(commands::cron_cmd::CronArgs {
            interval: 15,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressExplicitSync);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn register_is_suppressed_explicit_logging() {
        let b = behavior(Some(&Commands::Register(
            commands::register_cmd::RegisterArgs {
                server: "https://example.com".to_string(),
                force: false,
            },
        )));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressExplicitSync);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Internal worker ─────────────────────────────────────────────

    #[test]
    fn auto_sync_worker_is_suppressed_internal_logging() {
        let b = behavior(Some(&Commands::AutoSyncWorker {
            state_dir: PathBuf::from("/tmp/state"),
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressInternal);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Config/setup commands ───────────────────────────────────────

    #[test]
    fn update_is_suppressed_configuration_logging() {
        let b = behavior(Some(&Commands::Update {
            dry_run: false,
            locked: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressConfiguration);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn doctor_is_suppressed_configuration_logging() {
        let b = behavior(Some(&Commands::Doctor(commands::doctor_cmd::DoctorArgs {
            pet_file: None,
            compatibility: false,
            sync: false,
            check_shell: None,
            library: None,
            strict: false,
            report: commands::doctor_cmd::DiagnosticReportFormat::Human,
        })));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressConfiguration);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn completions_is_suppressed_configuration_logging() {
        let b = behavior(Some(&Commands::Completions {
            shell: clap_complete::Shell::Bash,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressConfiguration);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn shell_is_suppressed_configuration_logging() {
        let b = behavior(Some(&Commands::Shell {
            command: ShellCommands::Init {
                shell: commands::shell_cmd::ShellIntegration::Bash,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressConfiguration);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn keybindings_is_suppressed_configuration_logging() {
        let b = behavior(Some(&Commands::Keybindings));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressConfiguration);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Default / no subcommand ─────────────────────────────────────

    #[test]
    fn default_no_subcommand_is_allowed_logging() {
        let b = behavior(None);
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::Logging);
    }

    // ── Read-only commands avoid recovery/network side effects ──────

    #[test]
    fn read_only_commands_have_suppressed_recovery() {
        let read_only_cases: Vec<Option<Commands>> = vec![
            Some(Commands::Version),
            Some(Commands::List(commands::list_cmd::ListArgs {
                filter: None,
                config: None,
                library: None,
                json: false,
                csv: false,
                search_output: false,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
            })),
            Some(Commands::Status(commands::status_cmd::StatusArgs {
                json: false,
                sync_only: false,
            })),
            Some(Commands::Get(commands::get_cmd::GetArgs {
                id: None,
                description_exact: None,
                command_exact: None,
                query: None,
                library: None,
                field: None,
                raw: false,
                expanded: false,
                json: false,
                resolution: snip_it::selector::ResolutionPolicy::Unique,
                vars: None,
            })),
            Some(Commands::Validate(commands::validate_cmd::ValidateArgs {
                library: None,
                strict: false,
                json: false,
            })),
        ];
        for case in &read_only_cases {
            let b = behavior(case.as_ref());
            assert_ne!(
                b.recovery,
                StartupRecoveryPolicy::Allow,
                "read-only command should not allow recovery: {case:?}"
            );
        }
    }

    // ── Mutations allow pending recovery ────────────────────────────

    #[test]
    fn mutation_commands_allow_recovery() {
        let mutation_cases: Vec<Option<Commands>> = vec![
            Some(Commands::New(commands::new_cmd::NewArgs {
                command: None,
                tags: None,
                multiline: false,
                command_stdin: false,
                from_file: None,
                editor: false,
                description: None,
                config: None,
                library: None,
            })),
            Some(Commands::Run(commands::run_cmd::RunArgs {
                filter: None,
                sync: false,
                library: None,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
                id: None,
                description_exact: None,
                command_exact: None,
            })),
            Some(Commands::Clip(commands::clip_cmd::ClipArgs {
                filter: None,
                sync: false,
                library: None,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
                id: None,
                description_exact: None,
                command_exact: None,
            })),
            Some(Commands::Edit(commands::edit_cmd::EditArgs {
                library: None,
                output: None,
                output_stdin: false,
                clear_output: false,
                filter: None,
                id: None,
                description_exact: None,
                command_exact: None,
            })),
        ];
        for case in &mutation_cases {
            let b = behavior(case.as_ref());
            assert_eq!(
                b.recovery,
                StartupRecoveryPolicy::Allow,
                "mutation command should allow recovery: {case:?}"
            );
        }
    }

    // ── Explicit sync commands suppress startup auto-sync recovery ──

    #[test]
    fn explicit_sync_commands_suppress_recovery() {
        let sync_cases: Vec<Option<Commands>> = vec![
            Some(Commands::Sync { command: None }),
            Some(Commands::Cron(commands::cron_cmd::CronArgs {
                interval: 15,
            })),
            Some(Commands::Register(commands::register_cmd::RegisterArgs {
                server: "https://example.com".to_string(),
                force: false,
            })),
        ];
        for case in &sync_cases {
            let b = behavior(case.as_ref());
            assert_eq!(
                b.recovery,
                StartupRecoveryPolicy::SuppressExplicitSync,
                "sync command should suppress recovery: {case:?}"
            );
        }
    }
}
