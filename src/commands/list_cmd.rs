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
///
/// When `search_output` is true, the fuzzy filter also matches against
/// the snippet output/notes field (bounded to 512 chars for scoring).
pub fn run(
    filter: Option<String>,
    config: Option<PathBuf>,
    library: Option<String>,
    format: ListFormat,
    sort_opts: Option<crate::sort::SortOptions>,
    search_output: bool,
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

    let mut filtered: Vec<_> = if let Some(ref filter_str) = filter {
        snippets
            .snippets
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.deleted)
            .filter(|(_, s)| {
                let display = if search_output {
                    let output_summary =
                        crate::output::OutputPresentation::new(&s.output).for_scoring();
                    if output_summary.is_empty() {
                        format!("{} {}", s.description, s.command)
                    } else {
                        format!("{} {} {}", s.description, s.command, output_summary)
                    }
                } else {
                    format!("{} {}", s.description, s.command)
                };
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

    // Apply sort if specified
    if let Some(ref opts) = sort_opts {
        let indices: Vec<usize> = filtered.iter().map(|(i, _)| *i).collect();
        let usage_idx = crate::usage::UsageIndex::load();
        let usage_data: Vec<crate::usage::UsageData> = snippets
            .snippets
            .iter()
            .map(|s| usage_idx.get_usage(&s.id))
            .collect();
        let sorted_indices =
            crate::sort::rank_snippets(&indices, &snippets.snippets, None, Some(&usage_data), opts);
        let rank_map: std::collections::HashMap<usize, usize> = sorted_indices
            .iter()
            .enumerate()
            .map(|(rank, &idx)| (idx, rank))
            .collect();
        filtered.sort_by_key(|(i, _)| rank_map.get(i).copied().unwrap_or(usize::MAX));
    }

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
                if !s.output.is_empty() {
                    let presentation = crate::output::OutputPresentation::new(&s.output);
                    let summary = presentation.summary(80);
                    println!(
                        "{}: {}",
                        style("Output").with(Color::Yellow),
                        style(&summary).with(Color::White)
                    );
                }
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
    let field = if s
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@'))
    {
        format!("\t{s}")
    } else {
        s.to_string()
    };

    if field.contains(',')
        || s.contains('"')
        || field.contains('\n')
        || field.contains('\r')
        || field.contains('\t')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escape_quotes_commas_and_quotes() {
        assert_eq!(csv_escape("desc, with comma"), "\"desc, with comma\"");
        assert_eq!(csv_escape("echo \"quoted\""), "\"echo \"\"quoted\"\"\"");
    }

    #[test]
    fn csv_escape_prefixes_formula_values_before_quoting() {
        assert_eq!(csv_escape("=cmd"), "\"\t=cmd\"");
        assert_eq!(csv_escape("=cmd,with comma"), "\"\t=cmd,with comma\"");
    }
}
