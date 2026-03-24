use crate::commands::{get_library_path, load_snippets};
use crate::error::SnipResult;
use crossterm::style::{style, Color, Stylize};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;

pub fn run(
    filter: Option<String>,
    config: Option<PathBuf>,
    library: Option<String>,
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
            .filter(|(_, s)| {
                let display = format!("{} {}", s.description, s.command);
                matcher.fuzzy_match(&display, filter_str).is_some()
            })
            .collect()
    } else {
        snippets.snippets.iter().enumerate().collect()
    };

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
    Ok(())
}
