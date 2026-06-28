use crate::commands::{get_library_path, load_snippets};
use crate::error::SnipResult;
use crossterm::style::{Color, Stylize, style};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::path::PathBuf;

/// Output format for the `list` command.
#[derive(Debug, Clone, PartialEq)]
pub enum ListFormat {
    /// Human-readable table format (default).
    Default,
    /// JSON output for scripting.
    Json,
    /// CSV output for spreadsheet import.
    Csv,
}

/// Lists snippets from the library, optionally filtered and in a given format.
pub fn run(
    filter: Option<String>,
    config: Option<PathBuf>,
    library: Option<String>,
    format: ListFormat,
) -> SnipResult<()> {
    let snippets = if config.is_some() {
        load_snippets(&config)?
    } else {
        let lib_path = match get_library_path(library)? {
            Some(p) => p,
            None => {
                eprintln!("No library found. Create one with 'snp library create <name>'");
                return Ok(());
            }
        };
        crate::library::load_library(&lib_path)?
    };

    let matcher = SkimMatcherV2::default();

    let filtered: Vec<_> = if let Some(ref filter_str) = filter {
        snippets
            .snippets
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.deleted)
            .filter(|(_, s)| {
                let display = format!("{} {}", s.description, s.command);
                matcher.fuzzy_match(&display, filter_str).is_some()
            })
            .collect()
    } else {
        snippets
            .snippets
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.deleted)
            .collect()
    };

    match format {
        ListFormat::Json => {
            let items: Vec<_> = filtered
                .iter()
                .map(|(_, s)| {
                    serde_json::json!({
                        "description": s.description,
                        "command": s.command,
                        "output": s.output,
                        "tags": s.tags,
                        "folders": s.folders,
                        "favorite": s.favorite,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&items).map_err(|e| {
                    crate::error::SnipError::runtime_error(
                        "JSON serialization failed",
                        Some(&e.to_string()),
                    )
                })?
            );
        }
        ListFormat::Csv => {
            println!("description,command,output,tags,folders,favorite");
            for (_, s) in filtered {
                let tags = s.tags.join(";");
                let folders = s.folders.join(";");
                println!(
                    "{},{},{},{},{},{}",
                    csv_escape(&s.description),
                    csv_escape(&s.command),
                    csv_escape(&s.output),
                    csv_escape(&tags),
                    csv_escape(&folders),
                    s.favorite
                );
            }
        }
        ListFormat::Default => {
            for (_, s) in filtered {
                println!("{}", style("-----").with(Color::Blue));
                println!(
                    "{}: {}",
                    style(&s.description).with(Color::Green),
                    style(&s.command).with(Color::White)
                );
                println!(
                    "{}: {}",
                    style("Output").with(Color::Yellow),
                    style(&s.output).with(Color::White)
                );
                println!(
                    "{}: {}",
                    style("Tags").with(Color::Cyan),
                    style(s.tags.join(", ")).with(Color::White)
                );
            }
        }
    }
    Ok(())
}

fn csv_escape(s: &str) -> String {
    let escaped = if s.contains(',')
        || s.contains('"')
        || s.contains('\n')
        || s.contains('\r')
        || s.contains('\t')
    {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    };

    // Prefix formula-triggering characters to prevent CSV injection in spreadsheets.
    if let Some(first) = escaped.chars().next()
        && (first == '=' || first == '+' || first == '-' || first == '@')
    {
        return format!("\t{escaped}");
    }

    escaped
}
