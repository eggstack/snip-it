//! snp - A fast, terminal-based snippet manager.
//!
//! Features include fuzzy search, clipboard support, variable expansion,
//! TUI interface, and optional self-hosted sync with end-to-end encryption.

use std::path::PathBuf;
use std::sync::LazyLock;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use snip_it::CommandOutcome;
use snip_it::auto_sync::StartupRecoveryPolicy;
use snip_it::commands;
use snip_it::config;
use snip_it::error::SnipResult;
use snip_it::logging::{
    init_default_file_logging, init_default_logging, log_shutdown_info, log_startup_info,
    setup_panic_handler,
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
    /// List all snippets (l) — never executes
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
    /// Run a snippet via TUI selection (r) — executes via shell
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
        /// Match by exact snippet UUID (bypasses TUI)
        #[arg(long, conflicts_with_all = ["description_exact", "command_exact", "filter"])]
        id: Option<String>,
        /// Match by exact description (bypasses TUI)
        #[arg(long = "description-exact", conflicts_with_all = ["id", "command_exact", "filter"])]
        description_exact: Option<String>,
        /// Match by exact command text (bypasses TUI)
        #[arg(long = "command-exact", conflicts_with_all = ["id", "description_exact", "filter"])]
        command_exact: Option<String>,
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
        /// Match by exact snippet UUID (bypasses TUI)
        #[arg(long, conflicts_with_all = ["description_exact", "command_exact", "filter"])]
        id: Option<String>,
        /// Match by exact description (bypasses TUI)
        #[arg(long = "description-exact", conflicts_with_all = ["id", "command_exact", "filter"])]
        description_exact: Option<String>,
        /// Match by exact command text (bypasses TUI)
        #[arg(long = "command-exact", conflicts_with_all = ["id", "description_exact", "filter"])]
        command_exact: Option<String>,
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
        /// Match by exact snippet UUID (bypasses TUI for output editing)
        #[arg(long, conflicts_with_all = ["description_exact", "command_exact"])]
        id: Option<String>,
        /// Match by exact description (bypasses TUI)
        #[arg(long = "description-exact", conflicts_with_all = ["id", "command_exact"])]
        description_exact: Option<String>,
        /// Match by exact command text (bypasses TUI)
        #[arg(long = "command-exact", conflicts_with_all = ["id", "description_exact"])]
        command_exact: Option<String>,
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
    /// Repair configuration and library files
    #[command(alias = "rp")]
    Repair {
        /// Show planned repairs without making changes
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
        /// Apply safe repairs (creates backup first)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        apply: bool,
        /// Repair a specific library
        #[arg(short, long)]
        library: Option<String>,
        /// Output as JSON
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
    },
    /// Validate snippet data (read-only)
    #[command(alias = "val")]
    Validate {
        /// Validate a specific library
        #[arg(short, long)]
        library: Option<String>,
        /// Treat warnings as errors
        #[arg(long, action = clap::ArgAction::SetTrue)]
        strict: bool,
        /// Output as JSON
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
    },
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
    Backup {
        /// Output directory (default: ~/.config/snp/backups/\{timestamp\}/)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Include usage metadata in backup
        #[arg(long)]
        include_usage: bool,

        /// Include sync.toml in backup (API key redacted)
        #[arg(long)]
        include_sync_state: bool,
        /// Backup format
        #[arg(long, value_enum, default_value = "directory")]
        format: commands::backup_cmd::BackupFormat,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Restore from a backup snapshot
    Restore {
        /// Path to the backup directory
        #[arg(value_name = "BACKUP_DIR")]
        backup: PathBuf,
        /// Restore mode
        #[arg(long, value_enum, default_value = "merge")]
        mode: commands::restore_cmd::RestoreMode,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show auto-sync status (read-only)
    Status {
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync_only: bool,
    },
    /// Retrieve a snippet deterministically (no TUI, no execution)
    Get {
        /// Match by exact snippet UUID
        #[arg(long, conflicts_with_all = ["description_exact", "command_exact", "query"])]
        id: Option<String>,
        /// Match by exact description (case-insensitive)
        #[arg(long = "description-exact", conflicts_with_all = ["id", "command_exact", "query"])]
        description_exact: Option<String>,
        /// Match by exact command text (case-insensitive)
        #[arg(long = "command-exact", conflicts_with_all = ["id", "description_exact", "query"])]
        command_exact: Option<String>,
        /// Fuzzy query match
        #[arg(short, long, conflicts_with_all = ["id", "description_exact", "command_exact"])]
        query: Option<String>,
        /// Library scope (name, or "all" for all libraries)
        #[arg(short, long)]
        library: Option<String>,
        /// Output only a specific field
        #[arg(long, value_enum)]
        field: Option<commands::get_cmd::GetField>,
        /// Output raw stored bytes (no variable expansion, no trailing newline)
        #[arg(long, conflicts_with = "expanded")]
        raw: bool,
        /// Output with variables expanded using defaults
        #[arg(long, conflicts_with = "raw")]
        expanded: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Resolution policy for multiple matches
        #[arg(long, value_enum, default_value_t = snip_it::selector::ResolutionPolicy::Unique)]
        resolution: snip_it::selector::ResolutionPolicy,
        /// Explicit variable assignment (repeatable: --var host=example.com --var env=prod)
        #[arg(long = "var", value_name = "KEY=VALUE", action = clap::ArgAction::Append)]
        vars: Option<Vec<String>>,
    },
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
        shell: ShellIntegration,
    },
}

#[derive(Debug, Subcommand)]
enum DataCommands {
    /// Validate snippet data (read-only)
    #[command(alias = "v")]
    Validate {
        /// Validate a specific library
        #[arg(short, long)]
        library: Option<String>,
        /// Treat warnings as errors
        #[arg(long, action = clap::ArgAction::SetTrue)]
        strict: bool,
        /// Output as JSON
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
    },
    /// Create a secret-free backup snapshot
    #[command(alias = "b")]
    Backup {
        /// Output directory (default: ~/.config/snp/backups/\{timestamp\}/)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Include usage metadata in backup
        #[arg(long)]
        include_usage: bool,
        /// Include sync.toml in backup (API key redacted)
        #[arg(long)]
        include_sync_state: bool,
        /// Backup format
        #[arg(long, value_enum, default_value = "directory")]
        format: commands::backup_cmd::BackupFormat,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Restore from a backup snapshot
    #[command(alias = "r")]
    Restore {
        /// Path to the backup directory
        #[arg(value_name = "BACKUP_DIR")]
        backup: PathBuf,
        /// Restore mode
        #[arg(long, value_enum, default_value = "merge")]
        mode: commands::restore_cmd::RestoreMode,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Repair configuration and library files
    #[command(alias = "r")]
    Repair {
        /// Show planned repairs without making changes
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
        /// Apply safe repairs (creates backup first)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        apply: bool,
        /// Repair a specific library
        #[arg(short, long)]
        library: Option<String>,
        /// Output as JSON
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
    },
    /// Show auto-sync status (read-only)
    #[command(alias = "s")]
    Status {
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        sync_only: bool,
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

/// Map a `RepairExitStatus` to the appropriate process exit code.
/// Clean/DryRun/Repaired → 0 (implicit), UnsafeOnly → 2, PartialFailure → 1.
fn exit_on_repair_status(status: commands::repair_cmd::RepairExitStatus) {
    match status {
        commands::repair_cmd::RepairExitStatus::Clean
        | commands::repair_cmd::RepairExitStatus::Repaired
        | commands::repair_cmd::RepairExitStatus::DryRun => {}
        commands::repair_cmd::RepairExitStatus::PartialFailure => {
            std::process::exit(snip_it::outcome::exit_code::GENERAL_ERROR);
        }
        commands::repair_cmd::RepairExitStatus::UnsafeOnly => {
            std::process::exit(snip_it::outcome::exit_code::USAGE_ERROR);
        }
    }
}

fn dispatch_command(cli: Option<Commands>) -> SnipResult<CommandOutcome> {
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
            id,
            description_exact,
            command_exact,
        }) => {
            if id.is_some() || description_exact.is_some() || command_exact.is_some() {
                let result = snip_it::selector::resolve_exact_target(
                    library,
                    id,
                    description_exact,
                    command_exact,
                )?;
                let outcome = match result {
                    snip_it::selector::SelectionResult::One(m) => {
                        let outcome = commands::run_cmd::run_exact(
                            &m.snippet,
                            sync,
                            sync.then_some(&RUNTIME),
                        )?;
                        match outcome {
                            CommandOutcome::ExecutionFailed { child_code } => {
                                std::process::exit(child_code.unwrap_or(8));
                            }
                            CommandOutcome::Cancelled => {
                                return Ok(CommandOutcome::Cancelled);
                            }
                            _ => snip_it::outcome::CliOutcome::Success,
                        }
                    }
                    snip_it::selector::SelectionResult::Ambiguous(identities) => {
                        for identity in &identities {
                            eprintln!(
                                "  {} - {} ({})",
                                identity.id, identity.description, identity.library_name
                            );
                        }
                        snip_it::outcome::CliOutcome::Ambiguous
                    }
                    _ => snip_it::outcome::CliOutcome::NotFound,
                };
                return match outcome {
                    snip_it::outcome::CliOutcome::Success => Ok(CommandOutcome::Success),
                    snip_it::outcome::CliOutcome::Cancelled => Ok(CommandOutcome::Cancelled),
                    _ => {
                        std::process::exit(outcome.exit_code());
                    }
                };
            } else {
                let sort_opts = snip_it::sort::SortOptions {
                    mode: sort,
                    favorites_first,
                };
                let outcome = commands::run_cmd::run(
                    filter,
                    sync,
                    library,
                    Some(sort_opts),
                    sync.then_some(&RUNTIME),
                )?;
                match outcome {
                    CommandOutcome::ExecutionFailed { child_code } => {
                        std::process::exit(child_code.unwrap_or(8));
                    }
                    CommandOutcome::Cancelled => {
                        return Ok(CommandOutcome::Cancelled);
                    }
                    _ => {}
                }
            }
        }
        Some(Commands::Clip {
            filter,
            sync,
            library,
            sort,
            favorites_first,
            id,
            description_exact,
            command_exact,
        }) => {
            if id.is_some() || description_exact.is_some() || command_exact.is_some() {
                let result = snip_it::selector::resolve_exact_target(
                    library,
                    id,
                    description_exact,
                    command_exact,
                )?;
                let outcome = match result {
                    snip_it::selector::SelectionResult::One(m) => {
                        commands::clip_cmd::run_exact(&m.snippet, sync, sync.then_some(&RUNTIME))?;
                        snip_it::outcome::CliOutcome::Success
                    }
                    snip_it::selector::SelectionResult::Ambiguous(identities) => {
                        for identity in &identities {
                            eprintln!(
                                "  {} - {} ({})",
                                identity.id, identity.description, identity.library_name
                            );
                        }
                        snip_it::outcome::CliOutcome::Ambiguous
                    }
                    _ => snip_it::outcome::CliOutcome::NotFound,
                };
                return match outcome {
                    snip_it::outcome::CliOutcome::Success => Ok(CommandOutcome::Success),
                    snip_it::outcome::CliOutcome::Cancelled => Ok(CommandOutcome::Cancelled),
                    _ => {
                        std::process::exit(outcome.exit_code());
                    }
                };
            } else {
                let sort_opts = snip_it::sort::SortOptions {
                    mode: sort,
                    favorites_first,
                };
                commands::clip_cmd::run(
                    filter,
                    sync,
                    library,
                    None,
                    Some(sort_opts),
                    sync.then_some(&RUNTIME),
                )?;
            }
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
            commands::search_cmd::run(
                filter,
                sync,
                library,
                None,
                Some(sort_opts),
                sync.then_some(&RUNTIME),
            )?;
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
            );
        }
        Some(Commands::Edit {
            library,
            output,
            output_stdin,
            clear_output,
            filter,
            id,
            description_exact,
            command_exact,
        }) => {
            let has_output_flags = output.is_some() || output_stdin || clear_output;
            let has_exact = id.is_some() || description_exact.is_some() || command_exact.is_some();
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
                if has_exact {
                    let lib_for_edit = library.clone();
                    let result = snip_it::selector::resolve_exact_target(
                        library,
                        id,
                        description_exact,
                        command_exact,
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
                            for identity in &identities {
                                eprintln!(
                                    "  {} - {} ({})",
                                    identity.id, identity.description, identity.library_name
                                );
                            }
                            snip_it::outcome::CliOutcome::Ambiguous
                        }
                        _ => snip_it::outcome::CliOutcome::NotFound,
                    };
                    return match outcome {
                        snip_it::outcome::CliOutcome::Success => Ok(CommandOutcome::Success),
                        snip_it::outcome::CliOutcome::Cancelled => Ok(CommandOutcome::Cancelled),
                        _ => {
                            std::process::exit(outcome.exit_code());
                        }
                    };
                } else {
                    let filter_str = filter.ok_or_else(|| {
                        snip_it::error::SnipError::runtime_error(
                            "--filter is required when using --output, --output-stdin, or --clear-output",
                            None,
                        )
                    })?;
                    commands::edit_cmd::run_edit_output(library, filter_str, output_value)?;
                }
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
        Some(Commands::Repair {
            dry_run,
            apply,
            library,
            json,
        }) => {
            let status = commands::repair_cmd::run(dry_run, apply, library, json)?;
            exit_on_repair_status(status);
        }
        Some(Commands::Validate {
            library,
            strict,
            json,
        }) => {
            commands::validate_cmd::run(library, strict, json)?;
        }
        Some(Commands::Backup {
            output,
            include_usage,
            include_sync_state,
            format,
            json,
        }) => {
            commands::backup_cmd::run(output, include_usage, include_sync_state, format, json)?;
        }
        Some(Commands::Restore { backup, mode, json }) => {
            commands::restore_cmd::run(backup, mode, json)?;
        }
        Some(Commands::Status { json, sync_only }) => {
            commands::status_cmd::run(json, sync_only)?;
        }
        Some(Commands::Data { command }) => match command {
            DataCommands::Validate {
                library,
                strict,
                json,
            } => {
                commands::validate_cmd::run(library, strict, json)?;
            }
            DataCommands::Backup {
                output,
                include_usage,
                include_sync_state,
                format,
                json,
            } => {
                commands::backup_cmd::run(output, include_usage, include_sync_state, format, json)?;
            }
            DataCommands::Restore { backup, mode, json } => {
                commands::restore_cmd::run(backup, mode, json)?;
            }
            DataCommands::Repair {
                dry_run,
                apply,
                library,
                json,
            } => {
                let status = commands::repair_cmd::run(dry_run, apply, library, json)?;
                exit_on_repair_status(status);
            }
            DataCommands::Status { json, sync_only } => {
                commands::status_cmd::run(json, sync_only)?;
            }
        },
        Some(Commands::Get {
            id,
            description_exact,
            command_exact,
            query,
            library,
            field,
            raw,
            expanded,
            json,
            resolution,
            vars,
        }) => {
            let outcome = commands::get_cmd::run(
                id,
                description_exact,
                command_exact,
                query,
                library,
                field,
                raw,
                expanded,
                json,
                resolution,
                vars,
            )?;
            return match outcome {
                snip_it::outcome::CliOutcome::Success => Ok(CommandOutcome::Success),
                snip_it::outcome::CliOutcome::Cancelled => Ok(CommandOutcome::Cancelled),
                _ => {
                    std::process::exit(outcome.exit_code());
                }
            };
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
    Ok(CommandOutcome::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupServices {
    Minimal,
    Logging,
    LoggingAndAudit,
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
            | Commands::List { .. }
            | Commands::Search { .. }
            | Commands::Select { .. }
            | Commands::Status { .. }
            | Commands::Get { .. }
            | Commands::Validate { .. }
            | Commands::Backup { .. }
            | Commands::Library {
                command: LibraryCommands::List | LibraryCommands::Show { .. },
            },
        ) => (
            StartupRecoveryPolicy::SuppressReadOnly,
            StartupServices::Minimal,
        ),

        // ── Dry-run / read-only modes of otherwise-mutating commands ─
        Some(
            Commands::Restore {
                mode: commands::restore_cmd::RestoreMode::DryRun,
                ..
            }
            | Commands::Import {
                command: ImportSubcommands::Pet { dry_run: true, .. },
            }
            | Commands::Repair { dry_run: true, .. },
        ) => (
            StartupRecoveryPolicy::SuppressReadOnly,
            StartupServices::Minimal,
        ),

        // ── Data subcommand group ───────────────────────────────────
        Some(Commands::Data { command }) => match command {
            DataCommands::Validate { .. }
            | DataCommands::Status { .. }
            | DataCommands::Backup { .. } => (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            ),
            DataCommands::Restore {
                mode: commands::restore_cmd::RestoreMode::DryRun,
                ..
            } => (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            ),
            DataCommands::Repair { dry_run: true, .. } => (
                StartupRecoveryPolicy::SuppressReadOnly,
                StartupServices::Minimal,
            ),
            DataCommands::Repair { .. } | DataCommands::Restore { .. } => (
                StartupRecoveryPolicy::Allow,
                StartupServices::LoggingAndAudit,
            ),
        },

        // ── Mutation commands: allow recovery, full logging+audit ───
        Some(
            Commands::New { .. }
            | Commands::Run { .. }
            | Commands::Clip { .. }
            | Commands::Edit { .. }
            | Commands::Import { .. }
            | Commands::Repair { .. }
            | Commands::Restore { .. }
            | Commands::Premade { .. }
            | Commands::Library {
                command:
                    LibraryCommands::Create { .. }
                    | LibraryCommands::Delete { .. }
                    | LibraryCommands::SetPrimary { .. },
            },
        ) => (
            StartupRecoveryPolicy::Allow,
            StartupServices::LoggingAndAudit,
        ),

        // ── Explicit sync commands: suppress recovery, logging only ──
        Some(Commands::Sync { .. } | Commands::Cron { .. } | Commands::Register { .. }) => (
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
            | Commands::Doctor { .. }
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
        StartupServices::LoggingAndAudit => init_default_logging(),
    }
    if behavior.services != StartupServices::Minimal {
        log_startup_info();
    }

    if snip_it::auto_sync::should_attempt_auto_sync_recovery_for_policy(Some(behavior.recovery)) {
        snip_it::auto_sync::startup_recover_pending();
    }

    match dispatch_command(cli.command) {
        Ok(CommandOutcome::Success) => {}
        Ok(CommandOutcome::Cancelled) => {
            if behavior.services != StartupServices::Minimal {
                log_shutdown_info();
            }
            std::process::exit(4);
        }
        Ok(CommandOutcome::ExecutionFailed { child_code }) => {
            if behavior.services != StartupServices::Minimal {
                log_shutdown_info();
            }
            std::process::exit(child_code.unwrap_or(8));
        }
        Ok(_) => {}
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

    // ── Read-only commands ──────────────────────────────────────────

    #[test]
    fn version_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Version));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn list_is_minimal_readonly() {
        let b = behavior(Some(&Commands::List {
            filter: None,
            config: None,
            library: None,
            json: false,
            csv: false,
            search_output: false,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn search_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Search {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn select_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Select {
            filter: None,
            query: None,
            library: None,
            raw: false,
            expanded: false,
            output_file: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn status_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Status {
            json: false,
            sync_only: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn get_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Get {
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
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn validate_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Validate {
            library: None,
            strict: false,
            json: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn backup_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Backup {
            output: None,
            include_usage: false,
            include_sync_state: false,
            format: commands::backup_cmd::BackupFormat::Directory,
            json: false,
        }));
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
        let b = behavior(Some(&Commands::Restore {
            backup: PathBuf::from("/tmp/backup"),
            mode: commands::restore_cmd::RestoreMode::DryRun,
            json: false,
        }));
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
        let b = behavior(Some(&Commands::Repair {
            dry_run: true,
            apply: false,
            library: None,
            json: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    // ── Data subcommand group ───────────────────────────────────────

    #[test]
    fn data_validate_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Validate {
                library: None,
                strict: false,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_status_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Status {
                json: false,
                sync_only: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_backup_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Backup {
                output: None,
                include_usage: false,
                include_sync_state: false,
                format: commands::backup_cmd::BackupFormat::Directory,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_restore_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Restore {
                backup: PathBuf::from("/tmp/b"),
                mode: commands::restore_cmd::RestoreMode::DryRun,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_repair_dryrun_is_minimal_readonly() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Repair {
                dry_run: true,
                apply: false,
                library: None,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressReadOnly);
        assert_eq!(b.services, StartupServices::Minimal);
    }

    #[test]
    fn data_repair_mutation_is_allowed() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Repair {
                dry_run: false,
                apply: true,
                library: None,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn data_restore_mutation_is_allowed() {
        let b = behavior(Some(&Commands::Data {
            command: DataCommands::Restore {
                backup: PathBuf::from("/tmp/b"),
                mode: commands::restore_cmd::RestoreMode::Merge,
                json: false,
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    // ── Mutation commands ───────────────────────────────────────────

    #[test]
    fn new_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::New {
            command: None,
            tags: None,
            multiline: false,
            command_stdin: false,
            from_file: None,
            editor: false,
            description: None,
            config: None,
            library: None,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn run_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Run {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
            id: None,
            description_exact: None,
            command_exact: None,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn clip_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Clip {
            filter: None,
            sync: false,
            library: None,
            sort: snip_it::sort::SnippetSort::Relevance,
            favorites_first: false,
            id: None,
            description_exact: None,
            command_exact: None,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn edit_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Edit {
            library: None,
            output: None,
            output_stdin: false,
            clear_output: false,
            filter: None,
            id: None,
            description_exact: None,
            command_exact: None,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
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
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn repair_mutation_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Repair {
            dry_run: false,
            apply: true,
            library: None,
            json: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn restore_mutation_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Restore {
            backup: PathBuf::from("/tmp/b"),
            mode: commands::restore_cmd::RestoreMode::Merge,
            json: false,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn premade_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Premade {
            command: PremadeCommands::List,
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn library_create_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::Create {
                name: "test".to_string(),
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
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
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
    }

    #[test]
    fn library_set_primary_is_allowed_logging_and_audit() {
        let b = behavior(Some(&Commands::Library {
            command: LibraryCommands::SetPrimary {
                name: "test".to_string(),
            },
        }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::Allow);
        assert_eq!(b.services, StartupServices::LoggingAndAudit);
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
        let b = behavior(Some(&Commands::Cron { interval: 15 }));
        assert_eq!(b.recovery, StartupRecoveryPolicy::SuppressExplicitSync);
        assert_eq!(b.services, StartupServices::Logging);
    }

    #[test]
    fn register_is_suppressed_explicit_logging() {
        let b = behavior(Some(&Commands::Register {
            server: "https://example.com".to_string(),
            force: false,
        }));
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
        let b = behavior(Some(&Commands::Doctor {
            pet_file: None,
            compatibility: false,
            sync: false,
            check_shell: None,
            library: None,
            strict: false,
            report: commands::doctor_cmd::DiagnosticReportFormat::Human,
        }));
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
                shell: ShellIntegration::Bash,
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
            Some(Commands::List {
                filter: None,
                config: None,
                library: None,
                json: false,
                csv: false,
                search_output: false,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
            }),
            Some(Commands::Search {
                filter: None,
                sync: false,
                library: None,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
            }),
            Some(Commands::Status {
                json: false,
                sync_only: false,
            }),
            Some(Commands::Get {
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
            }),
            Some(Commands::Validate {
                library: None,
                strict: false,
                json: false,
            }),
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
            Some(Commands::New {
                command: None,
                tags: None,
                multiline: false,
                command_stdin: false,
                from_file: None,
                editor: false,
                description: None,
                config: None,
                library: None,
            }),
            Some(Commands::Run {
                filter: None,
                sync: false,
                library: None,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
                id: None,
                description_exact: None,
                command_exact: None,
            }),
            Some(Commands::Clip {
                filter: None,
                sync: false,
                library: None,
                sort: snip_it::sort::SnippetSort::Relevance,
                favorites_first: false,
                id: None,
                description_exact: None,
                command_exact: None,
            }),
            Some(Commands::Edit {
                library: None,
                output: None,
                output_stdin: false,
                clear_output: false,
                filter: None,
                id: None,
                description_exact: None,
                command_exact: None,
            }),
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
            Some(Commands::Cron { interval: 15 }),
            Some(Commands::Register {
                server: "https://example.com".to_string(),
                force: false,
            }),
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
