use crate::commands::{get_library_path, init_library_manager, load_snippets, save_snippets};
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crossterm::style::{Color, Stylize, style};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const TAG_PROMPT_SENTINEL: &str = "__snp_prompt_tags__";
pub const MAX_COMMAND_STDIN_BYTES: usize = 16 * 1024 * 1024;

/// The source of a new snippet's command body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    /// A command supplied as the existing positional argument.
    Positional(String),
    /// A command read exactly from stdin.
    Stdin,
    /// A command entered through the existing single-line prompt.
    InteractivePrompt,
    /// A command entered through the existing two-blank-line prompt.
    MultilinePrompt,
    /// A command read from a file path.
    File(PathBuf),
    /// A command written in an external editor.
    Editor,
}

/// Resolved editor program and argument vector. The program is launched directly
/// without any shell evaluation so that arguments are passed through unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl EditorCommand {
    pub fn program_label(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }
}

/// Source identifier used in validation diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSourceKind {
    Stdin,
    File,
    Editor,
}

impl CommandSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            CommandSourceKind::Stdin => "stdin",
            CommandSourceKind::File => "file",
            CommandSourceKind::Editor => "editor",
        }
    }
}

/// Maximum size for any single command body captured through an exact source.
pub const MAX_EXACT_COMMAND_BYTES: usize = MAX_COMMAND_STDIN_BYTES;

/// Validate raw command bytes captured from an exact source (stdin, file, or
/// editor). All exact sources share the same rules so that the storage layer
/// never sees drift between acquisition modes.
///
/// The function decodes UTF-8, rejects NUL bytes, and rejects empty or
/// whitespace-only input. The original bytes are never modified — accepted
/// content is returned exactly as provided (including supplied trailing
/// newlines). Callers must pass `source` for diagnostics.
pub fn validate_exact_command_bytes(
    bytes: Vec<u8>,
    source: CommandSourceKind,
) -> SnipResult<String> {
    if bytes.len() > MAX_EXACT_COMMAND_BYTES {
        return Err(SnipError::runtime_error(
            "Command input is too large",
            Some(&format!(
                "{} command data is limited to {} MiB",
                source.label(),
                MAX_EXACT_COMMAND_BYTES / (1024 * 1024)
            )),
        ));
    }

    let command = String::from_utf8(bytes).map_err(|_| {
        SnipError::runtime_error(
            "Invalid command input",
            Some(&format!(
                "{} command data must be valid UTF-8",
                source.label()
            )),
        )
    })?;

    if command.contains('\0') {
        return Err(SnipError::runtime_error(
            "Invalid command input",
            Some(&format!(
                "{} command data cannot contain NUL bytes",
                source.label()
            )),
        ));
    }

    if command.trim().is_empty() {
        return Err(SnipError::runtime_error(
            "Empty command",
            Some(&format!("{} produced no command data", source.label())),
        ));
    }

    Ok(command)
}

/// Read exact command data from a reader without evaluating, trimming, or
/// appending a newline.
pub fn read_command_stdin<R: Read>(reader: R) -> SnipResult<String> {
    let mut bytes = Vec::new();
    let limit = (MAX_EXACT_COMMAND_BYTES as u64) + 1;
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read command from stdin", "stdin", e))?;

    validate_exact_command_bytes(bytes, CommandSourceKind::Stdin)
}

/// Read a command from a file path.
///
/// The resolved path must point at a regular file. Symlinks are followed and
/// their targets are validated; broken symlinks, directories, FIFOs, sockets,
/// and device nodes are rejected. See Workstream C in the Release 2 closure
/// pass for the deliberate policy choice. The file is read as-is with no
/// trimming, normalization, or execution.
pub fn read_file_command(path: &Path) -> SnipResult<String> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            SnipError::runtime_error(
                "File not found",
                Some(&format!("'{}' does not exist", path.display())),
            )
        } else {
            SnipError::io_error("read file metadata", path, e)
        }
    })?;

    if metadata.is_dir() {
        return Err(SnipError::runtime_error(
            "Path is a directory",
            Some(&format!("'{}' is a directory, not a file", path.display())),
        ));
    }

    if !metadata.is_file() {
        return Err(SnipError::runtime_error(
            "Unsupported file type",
            Some(&format!("'{}' is not a regular file", path.display())),
        ));
    }

    let file = std::fs::File::open(path)
        .map_err(|e| SnipError::io_error("open file for reading", path, e))?;

    let mut bytes = Vec::new();
    let limit = (MAX_EXACT_COMMAND_BYTES as u64) + 1;
    io::BufReader::new(file)
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read command from file", path, e))?;

    validate_exact_command_bytes(bytes, CommandSourceKind::File)
}

fn has_directory_component(editor: &str) -> bool {
    editor.contains('/') || (cfg!(windows) && editor.contains('\\')) || editor.starts_with('.')
}

/// Locate an editor binary on disk. Used by [`resolve_editor_spec`] when an
/// executable path is relative or bare, so we can report a clear error if the
/// user-supplied value is unrunnable.
fn resolve_editor(editor: &str) -> SnipResult<String> {
    let editor_path = Path::new(editor);

    if editor_path.is_absolute() {
        if !editor_path.exists() {
            return Err(SnipError::runtime_error(
                "Editor not found",
                Some(&format!(
                    "EDITOR '{editor}' does not exist. Set EDITOR to a valid editor path."
                )),
            ));
        }
        if !editor_path.is_file() {
            return Err(SnipError::runtime_error(
                "Editor is not a file",
                Some(&format!(
                    "EDITOR '{editor}' exists but is not a file (it may be a directory). \
                     Set EDITOR to a valid editor executable."
                )),
            ));
        }
        return Ok(editor.to_string());
    }

    if has_directory_component(editor) {
        let cwd = std::env::current_dir().map_err(|e| {
            SnipError::runtime_error(
                "Failed to get current directory",
                Some(&format!("Cannot resolve relative editor path: {e}")),
            )
        })?;
        let candidate = cwd.join(editor);
        if !candidate.exists() {
            return Err(SnipError::runtime_error(
                "Editor not found",
                Some(&format!(
                    "EDITOR '{}' does not exist relative to {}.",
                    editor,
                    cwd.display()
                )),
            ));
        }
        if !candidate.is_file() {
            return Err(SnipError::runtime_error(
                "Editor is not a file",
                Some(&format!(
                    "EDITOR '{}' exists but is not a file.",
                    candidate.display()
                )),
            ));
        }

        let canonical = candidate.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Editor path resolution failed",
                Some(&format!(
                    "Cannot resolve editor path '{}': {}",
                    candidate.display(),
                    e
                )),
            )
        })?;

        let canonical_cwd = cwd.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Current directory resolution failed",
                Some(&format!("Cannot canonicalize CWD: {e}")),
            )
        })?;

        if !canonical.starts_with(&canonical_cwd) {
            return Err(SnipError::runtime_error(
                "Editor path unsafe",
                Some(&format!(
                    "EDITOR '{editor}' resolves outside of current directory (possible symlink attack). Use an absolute path."
                )),
            ));
        }

        return Ok(candidate.to_string_lossy().into_owned());
    }

    let path_var = std::env::var("PATH").unwrap_or_default();

    for dir in path_var.split(if cfg!(windows) { ';' } else { ':' }) {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(editor);
        if candidate.exists() && candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
        #[cfg(windows)]
        {
            for ext in &[".exe", ".cmd", ".bat"] {
                let with_ext = Path::new(dir).join(format!("{}{}", editor, ext));
                if with_ext.exists() && with_ext.is_file() {
                    return Ok(with_ext.to_string_lossy().into_owned());
                }
            }
        }
    }

    Err(SnipError::runtime_error(
        "Editor not found",
        Some(&format!(
            "EDITOR '{editor}' is not an absolute path and could not be found in PATH. \
             Set EDITOR to an absolute path (e.g., /usr/bin/vim)."
        )),
    ))
}

/// Parse an editor command specification into a program and argument list
/// using shell-word semantics. The value is split without invoking a shell so
/// that arguments are passed through to the editor verbatim.
pub fn parse_editor_spec(spec: &str) -> SnipResult<EditorCommand> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(SnipError::runtime_error(
            "Editor command is empty",
            Some("the editor specification must contain a program"),
        ));
    }

    let parts = shell_words::split(trimmed).map_err(|e| {
        SnipError::runtime_error(
            "Invalid editor command",
            Some(&format!("could not parse editor specification: {e}")),
        )
    })?;

    if parts.is_empty() {
        return Err(SnipError::runtime_error(
            "Editor command is empty",
            Some("the editor specification must contain a program"),
        ));
    }

    let mut iter = parts.into_iter();
    let program = iter.next().expect("checked non-empty above").into();
    let args: Vec<OsString> = iter.map(OsString::from).collect();
    Ok(EditorCommand { program, args })
}

/// Resolve the editor spec to a runnable program path on disk, preserving
/// arguments. Precedence is `$VISUAL` if non-empty, else `$EDITOR` if
/// non-empty, else `vim`.
pub fn resolve_editor_spec() -> SnipResult<EditorCommand> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "vim".to_string());

    let parsed = parse_editor_spec(&raw)?;
    let program_str = parsed.program.to_string_lossy().into_owned();
    let resolved = resolve_editor(&program_str)?;
    Ok(EditorCommand {
        program: OsString::from(resolved),
        args: parsed.args,
    })
}

/// Open an editor to compose a command body.
///
/// Creates an atomic temp file via [`tempfile::Builder`] with private
/// permissions, launches the editor as a child process (no shell), reads the
/// result, validates it through [`validate_exact_command_bytes`], and lets
/// the `NamedTempFile` owner drop on return. The editor's working directory
/// is unchanged and the command body is never logged.
pub fn read_editor_command() -> SnipResult<String> {
    let editor = resolve_editor_spec()?;
    let editor_label = editor.program_label();

    let temp = tempfile::Builder::new()
        .prefix("snp-editor-")
        .suffix(".sh")
        .tempfile()
        .map_err(|e| SnipError::io_error("create editor temp file", "<tempfile>", e))?;
    let temp_path = temp.path().to_owned();

    let mut command = Command::new(&editor.program);
    command.args(&editor.args);
    command.arg(&temp_path);

    let status = command.status().map_err(|e| {
        SnipError::command_error(
            &editor_label,
            editor
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .chain(std::iter::once(temp_path.display().to_string()))
                .collect(),
            e,
        )
    })?;

    if !status.success() {
        return Err(SnipError::runtime_error(
            "Editor exited with error",
            Some(&format!(
                "editor '{}' exited with status {}",
                editor_label,
                status.code().unwrap_or(-1)
            )),
        ));
    }

    // Open the file again rather than holding a borrowed handle, so the
    // NamedTempFile owner can clean up the path on drop.
    let mut bytes = Vec::new();
    std::fs::File::open(&temp_path)
        .map_err(|e| SnipError::io_error("open editor output", &temp_path, e))?
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read editor output", &temp_path, e))?;

    validate_exact_command_bytes(bytes, CommandSourceKind::Editor)
}

/// Reads a multiline snippet command from any `Read` source, terminated
/// by two consecutive blank lines.
///
/// The terminator delimiter itself is consumed and does not appear in the
/// output. Content that needs to end with blank lines or contain the
/// two-consecutive-blank-line delimiter at the end cannot be represented
/// losslessly through this path. Use `--command-stdin`, `--from-file`, or
/// `--editor` when full byte fidelity is required.
pub fn read_multiline_from<R: io::BufRead>(mut reader: R) -> io::Result<String> {
    let mut lines = Vec::new();
    let mut prev_was_empty = false;

    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        if line.trim().is_empty() && prev_was_empty {
            break;
        }

        prev_was_empty = line.trim().is_empty();
        lines.push(line);
    }

    Ok(lines.join(""))
}

/// Reads a multiline snippet command from stdin (terminated by two blank lines).
///
/// `--multiline` is an interactive convenience mode terminated by two
/// consecutive blank lines. The terminator cannot itself appear in the body,
/// so multiline input is **not byte-exact** for content that needs trailing
/// blank lines or a final blank-line sequence. Use `--command-stdin`,
/// `--from-file`, or `--editor` when full fidelity is required.
pub fn read_multiline_command() -> io::Result<String> {
    read_multiline_from(io::stdin().lock())
}

fn read_prompt_line(prompt: &str, color: Color) -> SnipResult<String> {
    print!("{}", style(prompt).with(color));
    io::stdout().flush()?;
    let mut input = String::new();
    let bytes_read = io::stdin().read_line(&mut input)?;
    if bytes_read == 0 {
        return Err(SnipError::runtime_error(
            "Input cancelled",
            Some("No metadata was supplied"),
        ));
    }
    Ok(input.trim().to_string())
}

fn parse_tags(value: &str) -> Vec<String> {
    value
        .split([' ', ','])
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

/// Creates a new snippet with the given command and optional tags.
pub fn run(
    command: Option<String>,
    description: Option<String>,
    tags: Option<String>,
    multiline: bool,
    command_stdin: bool,
    from_file: Option<PathBuf>,
    editor: bool,
    config: Option<PathBuf>,
    library: Option<String>,
) -> SnipResult<()> {
    if command_stdin && description.is_none() {
        return Err(SnipError::runtime_error(
            "Description required for stdin ingestion",
            Some("use --description because stdin is reserved for command data"),
        ));
    }
    if command_stdin && tags.as_deref() == Some(TAG_PROMPT_SENTINEL) {
        return Err(SnipError::runtime_error(
            "Tags must be explicit for stdin ingestion",
            Some("provide tag values to --tags or omit --tags"),
        ));
    }

    let source = if command_stdin {
        CommandSource::Stdin
    } else if let Some(path) = from_file {
        CommandSource::File(path)
    } else if editor {
        CommandSource::Editor
    } else if multiline {
        CommandSource::MultilinePrompt
    } else if let Some(command) = command {
        CommandSource::Positional(command)
    } else {
        CommandSource::InteractivePrompt
    };

    // Resolve command data and metadata before touching library state. This
    // keeps malformed stdin input from triggering migration or persistence.
    let command = match source {
        CommandSource::Stdin => read_command_stdin(io::stdin().lock())?,
        CommandSource::File(path) => read_file_command(&path)?,
        CommandSource::Editor => read_editor_command()?,
        CommandSource::MultilinePrompt => {
            let raw = read_multiline_command()?;
            if raw.trim().is_empty() {
                return Err(SnipError::runtime_error(
                    "Empty command",
                    Some("--multiline produced no command data"),
                ));
            }
            raw
        }
        CommandSource::InteractivePrompt => read_prompt_line("Command> ", Color::Yellow)?,
        CommandSource::Positional(command) => {
            if command.is_empty() {
                read_prompt_line("Command> ", Color::Yellow)?
            } else {
                println!(
                    "{}",
                    style(format!("Command> {command}")).with(Color::Yellow)
                );
                command
            }
        }
    };

    let description = match description {
        Some(description) => description,
        None => read_prompt_line("Description> ", Color::Green)?,
    };

    let tags = match tags.as_deref() {
        None => Vec::new(),
        Some(TAG_PROMPT_SENTINEL) => parse_tags(&read_prompt_line("Tags> ", Color::Cyan)?),
        Some(value) => parse_tags(value),
    };

    let fallback_path = config.or_else(|| {
        let mgr = init_library_manager().ok()?;
        mgr.get_primary_library()
            .map(|l| mgr.get_libraries_dir().join(format!("{}.toml", l.filename)))
    });

    let lib_path = get_library_path(library)?;
    let mut snippets = if let Some(ref p) = lib_path {
        crate::library::load_library(p)?
    } else {
        load_snippets(&fallback_path)?
    };

    let mut new_snippet = Snippet::new(description, command, tags)?;
    // Stamp the device_id from sync settings so the server can identify the
    // originating device. Without this the server rejects the snippet during
    // sync validation ("Device ID is required").
    let sync_settings = crate::config::get_sync_settings();
    if !sync_settings.device_id.is_empty() {
        new_snippet.device_id = sync_settings.device_id.clone();
    }
    snippets.snippets.push(new_snippet);

    if let Some(ref p) = lib_path {
        crate::library::save_library(p, &snippets)?;
    } else {
        save_snippets(&snippets, &fallback_path)?;
    }

    // Auto-sync trigger: notify after successful local commit (Workstream B1).
    crate::auto_sync::notify_mutation(
        crate::auto_sync::MutationKind::SnippetCreate,
        crate::auto_sync::MutationOrigin::User,
    );

    println!("Snippet added");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_reader_preserves_command_bytes_and_newlines() {
        let command = "-n echo \"quoted\" | sed 's/x/y/'\n\n";
        assert_eq!(read_command_stdin(command.as_bytes()).unwrap(), command);
    }

    #[test]
    fn stdin_reader_rejects_invalid_utf8_and_nul() {
        assert!(read_command_stdin([0xff, 0xfe].as_slice()).is_err());
        assert!(read_command_stdin(b"echo\0secret".as_slice()).is_err());
    }

    #[test]
    fn stdin_reader_rejects_empty_and_whitespace_only() {
        assert!(read_command_stdin(b"".as_slice()).is_err());
        assert!(read_command_stdin(b"\n\n".as_slice()).is_err());
        assert!(read_command_stdin(b"   \n\t\n".as_slice()).is_err());
    }

    #[test]
    fn shared_validator_rejects_oversized_input() {
        let big = vec![b'a'; MAX_EXACT_COMMAND_BYTES + 1];
        assert!(validate_exact_command_bytes(big, CommandSourceKind::Stdin).is_err());
    }

    #[test]
    fn tags_are_split_like_the_existing_prompt() {
        assert_eq!(
            parse_tags("git, release deploy"),
            ["git", "release", "deploy"]
        );
    }

    #[test]
    fn file_reader_preserves_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("cmd.txt");
        std::fs::write(&path, "echo hello\n").unwrap();
        assert_eq!(read_file_command(&path).unwrap(), "echo hello\n");
    }

    #[test]
    fn file_reader_rejects_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(read_file_command(tmp.path()).is_err());
    }

    #[test]
    fn file_reader_rejects_missing_file() {
        let path = PathBuf::from("/nonexistent/file/path/command.txt");
        assert!(read_file_command(&path).is_err());
    }

    #[test]
    fn file_reader_rejects_invalid_utf8() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.txt");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read_file_command(&path).is_err());
    }

    #[test]
    fn file_reader_rejects_nul_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("nul.txt");
        std::fs::write(&path, b"echo\0secret").unwrap();
        assert!(read_file_command(&path).is_err());
    }

    #[test]
    fn file_reader_preserves_trailing_newlines() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("trailing.txt");
        std::fs::write(&path, "echo hi\n\n\n").unwrap();
        assert_eq!(read_file_command(&path).unwrap(), "echo hi\n\n\n");
    }

    #[test]
    fn file_reader_follows_symlink_to_regular_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("target.sh");
        std::fs::write(&target, "echo linked\n").unwrap();
        let link = tmp.path().join("link.sh");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        assert_eq!(read_file_command(&link).unwrap(), "echo linked\n");
    }

    #[test]
    fn file_reader_rejects_broken_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let link = tmp.path().join("broken.sh");
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("missing"), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&tmp.path().join("missing"), &link).unwrap();
        assert!(read_file_command(&link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_reader_rejects_block_device_via_metadata() {
        // Reject paths that resolve to special files via metadata. We use a
        // symlink to /dev/null which is a character device; this exercises
        // the same metadata check used to reject FIFOs without needing to
        // create one (which blocks on macOS due to capability restrictions).
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let link = tmp.path().join("devnull.sh");
        symlink("/dev/null", &link).unwrap();
        let result = read_file_command(&link);
        assert!(result.is_err(), "/dev/null must be rejected: {result:?}");
    }

    #[test]
    fn parse_editor_spec_supports_args_and_quoted_paths() {
        let cmd = parse_editor_spec("code --wait").unwrap();
        assert_eq!(cmd.program, "code");
        assert_eq!(cmd.args, vec![OsString::from("--wait")]);

        let cmd = parse_editor_spec(
            "\"/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code\" --wait",
        )
        .unwrap();
        assert_eq!(
            cmd.program,
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
        );
        assert_eq!(cmd.args, vec![OsString::from("--wait")]);
    }

    #[test]
    fn parse_editor_spec_rejects_empty_and_malformed() {
        assert!(parse_editor_spec("").is_err());
        assert!(parse_editor_spec("   ").is_err());
        assert!(parse_editor_spec("code \"unterminated").is_err());
    }

    #[test]
    fn editor_tempfile_uses_os_temp_dir_and_secure_permissions() {
        let temp = tempfile::Builder::new()
            .prefix("snp-editor-")
            .suffix(".sh")
            .tempfile()
            .unwrap();
        let path = temp.path();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("snp-editor-"),
            "temp file should use snp-editor- prefix: {name}"
        );
        let parent = path.parent().unwrap();
        assert_eq!(
            parent,
            std::env::temp_dir(),
            "temp file should be created in OS temp dir"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o077,
                0,
                "editor temp file must not be group/world readable (mode={:o})",
                mode
            );
        }
    }

    // --- Multiline from-reader tests (Workstream E) ---

    #[test]
    fn multiline_terminates_after_two_blank_lines() {
        let input = "echo line1\necho line2\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        // The two consecutive blank lines trigger termination. The first
        // blank line is included in output; the second (delimiter) is consumed.
        assert_eq!(result, "echo line1\necho line2\n\n");
    }

    #[test]
    fn multiline_handles_internal_single_blank() {
        let input = "echo before\n\necho after\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        // Single blank line is part of the content, not a delimiter.
        assert_eq!(result, "echo before\n\necho after\n\n");
    }

    #[test]
    fn multiline_trims_trailing_delimiter() {
        // Content "echo test\n" followed by two blank lines.
        // The delimiter consumes the second blank; the first blank
        // is part of the output.
        let input = "echo test\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        assert_eq!(result, "echo test\n\n");
    }

    #[test]
    fn multiline_cannot_represent_two_consecutive_blanks_at_end() {
        // A command that genuinely needs two trailing blank lines
        // cannot be represented through multiline. The second blank
        // line is consumed as the delimiter.
        let input = "echo body\n\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        // Line "echo body\n" → content. Line "\n" (blank 1) → content.
        // Line "\n" (blank 2) → delimiter start. Line "\n" → terminates.
        // Result: "echo body\n\n" — only one trailing blank preserved.
        assert_eq!(result, "echo body\n\n");
    }

    #[test]
    fn multiline_eof_before_delimiter() {
        // Input terminated by EOF without two blank lines.
        let input = "echo only\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        assert_eq!(result, "echo only\n");
    }

    #[test]
    fn multiline_leading_blank_line() {
        let input = "\necho after_blank\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        assert_eq!(result, "\necho after_blank\n\n");
    }

    #[test]
    fn multiline_whitespace_only_delimiter_lines() {
        // Lines that are whitespace-only (spaces, tabs) count as blank
        // for the delimiter check (trim().is_empty()).
        let input = "echo test\n  \t\n\n\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        // "  \t\n" is blank (trim is empty), then "\n" is blank → delimiter.
        // The whitespace-only line is content; the empty line after it
        // starts the delimiter, and the final blank terminates.
        assert_eq!(result, "echo test\n  \t\n");
    }

    #[test]
    fn multiline_single_line_no_delimiter() {
        let input = "echo single\n";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        assert_eq!(result, "echo single\n");
    }

    #[test]
    fn multiline_empty_input() {
        let input = "";
        let result = read_multiline_from(input.as_bytes()).unwrap();
        assert_eq!(result, "");
    }
}
