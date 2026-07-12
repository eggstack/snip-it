use crate::commands::{get_library_path, init_library_manager, load_snippets, save_snippets};
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crossterm::style::{Color, Stylize, style};
use std::io::{self, Read, Write};
use std::path::PathBuf;

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
}
