use crate::commands::{get_library_path, init_library_manager, load_snippets, save_snippets};
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crossterm::style::{Color, Stylize, style};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const TAG_PROMPT_SENTINEL: &str = "__snp_prompt_tags__";
const MAX_COMMAND_STDIN_BYTES: usize = 16 * 1024 * 1024;

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

/// Read exact command data from a reader without evaluating, trimming, or
/// appending a newline.
pub fn read_command_stdin<R: Read>(reader: R) -> SnipResult<String> {
    let mut bytes = Vec::new();
    let limit = (MAX_COMMAND_STDIN_BYTES as u64) + 1;
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read command from stdin", "stdin", e))?;

    if bytes.len() > MAX_COMMAND_STDIN_BYTES {
        return Err(SnipError::runtime_error(
            "Command input is too large",
            Some("stdin command data is limited to 16 MiB"),
        ));
    }

    let command = String::from_utf8(bytes).map_err(|_| {
        SnipError::runtime_error(
            "Invalid command input",
            Some("stdin command data must be valid UTF-8"),
        )
    })?;

    if command.contains('\0') {
        return Err(SnipError::runtime_error(
            "Invalid command input",
            Some("stdin command data cannot contain NUL bytes"),
        ));
    }

    Ok(command)
}

/// Read a command from a file path.
///
/// Rejects directories, missing files, invalid UTF-8, and NUL bytes. The file
/// is read as-is with no trimming, normalization, or execution.
pub fn read_file_command(path: &Path) -> SnipResult<String> {
    if path.is_dir() {
        return Err(SnipError::runtime_error(
            "Path is a directory",
            Some(&format!("'{}' is a directory, not a file", path.display())),
        ));
    }
    if !path.exists() {
        return Err(SnipError::runtime_error(
            "File not found",
            Some(&format!("'{}' does not exist", path.display())),
        ));
    }

    let mut bytes = Vec::new();
    let file = std::fs::File::open(path)
        .map_err(|e| SnipError::io_error("open file for reading", path, e))?;
    let limit = (MAX_COMMAND_STDIN_BYTES as u64) + 1;
    io::BufReader::new(file)
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read command from file", path, e))?;

    if bytes.len() > MAX_COMMAND_STDIN_BYTES {
        return Err(SnipError::runtime_error(
            "File too large",
            Some("file command data is limited to 16 MiB"),
        ));
    }

    let command = String::from_utf8(bytes).map_err(|_| {
        SnipError::runtime_error(
            "Invalid command input",
            Some("file content must be valid UTF-8"),
        )
    })?;

    if command.contains('\0') {
        return Err(SnipError::runtime_error(
            "Invalid command input",
            Some("file content cannot contain NUL bytes"),
        ));
    }

    Ok(command)
}

fn has_directory_component(editor: &str) -> bool {
    editor.contains('/') || (cfg!(windows) && editor.contains('\\')) || editor.starts_with('.')
}

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

/// Open an editor to compose a command body.
///
/// Creates a temp file with restrictive permissions, launches the editor, reads
/// the result, and cleans up. Returns an error if the editor exits nonzero or
/// the content is empty after trimming trailing newlines.
pub fn read_editor_command() -> SnipResult<String> {
    let editor_env = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let resolved = resolve_editor(&editor_env)?;

    let temp_dir = std::env::temp_dir();
    let temp_name = format!(
        "snp-editor-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let temp_path = temp_dir.join(&temp_name);

    // Create with restrictive permissions
    {
        let f = std::fs::File::create(&temp_path)
            .map_err(|e| SnipError::io_error("create temp file", &temp_path, e))?;
        // Set 0600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| SnipError::io_error("set temp file permissions", &temp_path, e))?;
        }
        drop(f);
    }

    let _guard = crate::utils::tempfile_guard::TempFileGuard::new(temp_path.clone());

    let status = Command::new(&resolved)
        .arg(&temp_path)
        .status()
        .map_err(|e| {
            SnipError::command_error(&resolved, vec![temp_path.display().to_string()], e)
        })?;

    if !status.success() {
        return Err(SnipError::runtime_error(
            "Editor exited with error",
            Some(&format!(
                "editor '{}' exited with status {}",
                resolved,
                status.code().unwrap_or(-1)
            )),
        ));
    }

    let mut content = String::new();
    std::fs::File::open(&temp_path)
        .map_err(|e| SnipError::io_error("open editor output", &temp_path, e))?
        .read_to_string(&mut content)
        .map_err(|e| SnipError::io_error("read editor output", &temp_path, e))?;

    // Trim trailing newlines — empty after trimming means the user cancelled
    let trimmed = content.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return Err(SnipError::runtime_error(
            "Empty command",
            Some("editor produced no content; command not saved"),
        ));
    }

    Ok(content)
}

/// Reads a multiline snippet command from stdin (terminated by two blank lines).
pub fn read_multiline_command() -> io::Result<String> {
    let mut lines = Vec::new();
    let mut prev_was_empty = false;

    loop {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;

        if line.trim().is_empty() && prev_was_empty {
            break;
        }

        prev_was_empty = line.trim().is_empty();
        lines.push(line);
    }

    Ok(lines.join(""))
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
        CommandSource::MultilinePrompt => read_multiline_command()?,
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

    let new_snippet = Snippet::new(description, command, tags)?;
    snippets.snippets.push(new_snippet);

    if let Some(ref p) = lib_path {
        crate::library::save_library(p, &snippets)?;
    } else {
        save_snippets(&snippets, &fallback_path)?;
    }
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
}
