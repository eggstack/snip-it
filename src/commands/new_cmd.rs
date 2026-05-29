use crate::commands::{get_library_path, init_library_manager, load_snippets, save_snippets};
use crate::error::SnipResult;
use crate::library::Snippet;
use crossterm::style::{style, Color, Stylize};
use std::io::{self, Write};
use std::path::PathBuf;

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

pub fn run(
    command: String,
    tags: bool,
    multiline: bool,
    config: Option<PathBuf>,
    library: Option<String>,
) -> SnipResult<()> {
    let fallback_path = config.or_else(|| {
        let mgr = init_library_manager().ok()?;
        mgr.get_primary_library()
            .map(|l| mgr.get_libraries_dir().join(format!("{}.toml", l.filename)))
    });

    let command = if multiline {
        read_multiline_command()?
    } else if command.is_empty() {
        print!("{}", style("Command> ").with(Color::Yellow));
        io::stdout().flush()?;
        let mut cmd_input = String::new();
        io::stdin().read_line(&mut cmd_input)?;
        cmd_input.trim().to_string()
    } else {
        println!(
            "{}",
            style(format!("Command> {}", command)).with(Color::Yellow)
        );
        command
    };

    print!("{}", style("Description> ").with(Color::Green));
    io::stdout().flush()?;

    let mut description = String::new();
    io::stdin().read_line(&mut description)?;
    let description = description.trim().to_string();

    let tags: Vec<String> = if tags {
        print!("{}", style("Tags> ").with(Color::Cyan));
        io::stdout().flush()?;
        let mut tags_input = String::new();
        io::stdin().read_line(&mut tags_input)?;
        tags_input
            .split([' ', ','])
            .map(|s| s.trim().to_string())
            .collect()
    } else {
        Vec::new()
    };

    let lib_path = get_library_path(library)?;
    let mut snippets = if let Some(ref p) = lib_path {
        crate::library::load_library(p)?
    } else {
        load_snippets(&fallback_path)?
    };

    snippets
        .snippets
        .push(Snippet::new(description, command, tags));

    if let Some(ref p) = lib_path {
        crate::library::save_library(p, &snippets)?;
    } else {
        save_snippets(&snippets, &fallback_path)?;
    }
    println!("Snippet added");
    Ok(())
}
