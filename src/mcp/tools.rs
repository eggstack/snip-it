//! Read-only MCP tool implementations.

use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, LibraryMeta, Snippet};
use crate::selector::{ResolutionPolicy, SelectionResult, SnippetSelector};
use crate::sort::{SnippetSort, SortOptions, rank_snippets};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;

#[derive(Debug, Clone)]
struct LibrarySource {
    name: String,
    library_id: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct LoadedSnippet {
    snippet: Snippet,
    library: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    library: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArguments {
    query: String,
    library: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArguments {
    id: Option<String>,
    description: Option<String>,
    library: Option<String>,
}

pub fn list(arguments: Option<&Value>) -> SnipResult<Value> {
    let args = parse_arguments::<ListArguments>(arguments)?;
    let limit = bounded_limit(args.limit)?;
    let snippets = load_snippets(args.library.as_deref())?;
    let results = snippets
        .into_iter()
        .filter(|entry| !entry.snippet.deleted)
        .take(limit)
        .map(|entry| snippet_json(&entry))
        .collect::<Vec<_>>();
    Ok(json!({ "snippets": results }))
}

pub fn search(arguments: Option<&Value>) -> SnipResult<Value> {
    let args = parse_arguments::<SearchArguments>(arguments)?;
    let limit = bounded_limit(args.limit)?;
    let snippets = load_snippets(args.library.as_deref())?;

    let matcher = SkimMatcherV2::default();
    let mut source = Vec::new();
    let mut scores = HashMap::new();
    for entry in snippets {
        if entry.snippet.deleted {
            continue;
        }
        let index = source.len();
        if args.query.is_empty() {
            source.push(entry);
            continue;
        }
        let display = format!("{} {}", entry.snippet.description, entry.snippet.command);
        if let Some(score) = matcher.fuzzy_match(&display, &args.query) {
            scores.insert(index, score);
            source.push(entry);
        }
    }

    let snippets_only: Vec<Snippet> = source.iter().map(|entry| entry.snippet.clone()).collect();
    let indices: Vec<usize> = (0..snippets_only.len()).collect();
    let ranked = rank_snippets(
        &indices,
        &snippets_only,
        Some(&scores),
        None,
        &SortOptions {
            mode: SnippetSort::Relevance,
            favorites_first: false,
        },
    );
    let results = ranked
        .into_iter()
        .take(limit)
        .map(|index| snippet_json(&source[index]))
        .collect::<Vec<_>>();
    Ok(json!({ "snippets": results }))
}

pub fn get(arguments: Option<&Value>) -> SnipResult<Value> {
    let args = parse_arguments::<GetArguments>(arguments)?;
    let has_id = args.id.is_some();
    let has_description = args.description.is_some();
    if has_id == has_description {
        return Err(invalid_params(
            "Provide exactly one of 'id' or 'description'",
        ));
    }

    let sources = library_sources(args.library.as_deref())?;
    let mut matches = Vec::new();
    for source in sources {
        let snippets = crate::library::load_library(&source.path)?;
        let mut selector = SnippetSelector::new(ResolutionPolicy::All);
        if let Some(id) = &args.id {
            selector = selector.with_id(id.clone());
        }
        if let Some(description) = &args.description {
            selector = selector.with_description_exact(description.clone());
        }
        match selector.resolve(&snippets, &source.path, &source.name, &source.library_id)? {
            SelectionResult::One(m) => matches.push(LoadedSnippet {
                snippet: m.snippet,
                library: m.library_name,
            }),
            SelectionResult::Many(ms) => matches.extend(ms.into_iter().map(|m| LoadedSnippet {
                snippet: m.snippet,
                library: m.library_name,
            })),
            SelectionResult::NotFound | SelectionResult::Ambiguous(_) => {}
        }
    }

    matches.sort_by(|a, b| {
        a.library
            .to_lowercase()
            .cmp(&b.library.to_lowercase())
            .then_with(|| {
                a.snippet
                    .description
                    .to_lowercase()
                    .cmp(&b.snippet.description.to_lowercase())
            })
            .then_with(|| a.snippet.id.cmp(&b.snippet.id))
    });

    match matches.as_slice() {
        [] => Ok(json!({
            "error": "not_found",
            "message": "No matching snippet was found",
            "matches": []
        })),
        [match_] => Ok(snippet_json(match_)),
        many => Ok(json!({
            "error": "ambiguous",
            "message": "The selector matched more than one snippet",
            "matches": many.iter().map(snippet_identity_json).collect::<Vec<_>>()
        })),
    }
}

pub fn is_error(result: &Value) -> bool {
    result.get("error").is_some()
}

fn parse_arguments<T>(arguments: Option<&Value>) -> SnipResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let value = arguments.cloned().unwrap_or_else(|| json!({}));
    if !value.is_object() {
        return Err(invalid_params("Tool arguments must be a JSON object"));
    }
    serde_json::from_value(value)
        .map_err(|error| invalid_params(&format!("Invalid tool arguments: {error}")))
}

fn bounded_limit(limit: Option<usize>) -> SnipResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(invalid_params(&format!(
            "'limit' must be between 1 and {MAX_LIMIT}"
        )));
    }
    Ok(limit)
}

fn load_snippets(library: Option<&str>) -> SnipResult<Vec<LoadedSnippet>> {
    let sources = library_sources(library)?;
    let mut result = Vec::new();
    for source in sources {
        let snippets = crate::library::load_library(&source.path)?;
        result.extend(snippets.snippets.into_iter().map(|snippet| LoadedSnippet {
            snippet,
            library: source.name.clone(),
        }));
    }
    Ok(result)
}

/// Resolve read-only library sources without calling `ensure_library_mode`.
/// That method can migrate and write a legacy file, while MCP reads should be
/// side-effect free. Legacy single-file mode is represented as the implicit
/// `snippets` library for compatibility with normal CLI resolution.
fn library_sources(library: Option<&str>) -> SnipResult<Vec<LibrarySource>> {
    let manager = LibraryManager::new()?;
    if manager.is_single_file_mode() {
        let path = LibraryManager::get_default_snippets_path();
        let available = path.exists();
        return match library {
            None | Some("all") if available => Ok(vec![LibrarySource {
                name: "snippets".to_string(),
                library_id: String::new(),
                path,
            }]),
            Some("snippets") if available => Ok(vec![LibrarySource {
                name: "snippets".to_string(),
                library_id: String::new(),
                path,
            }]),
            Some("all") | None => Ok(Vec::new()),
            Some(name) => Err(library_not_found(name)),
        };
    }

    let make_source = |meta: &LibraryMeta| LibrarySource {
        name: meta.filename.clone(),
        library_id: meta.library_id.clone(),
        path: manager
            .get_libraries_dir()
            .join(format!("{}.toml", meta.filename)),
    };

    match library {
        Some("all") => Ok(manager
            .list_libraries()
            .into_iter()
            .map(make_source)
            .collect()),
        Some(name) => manager
            .get_library_by_filename(name)
            .map(|meta| vec![make_source(meta)])
            .ok_or_else(|| library_not_found(name)),
        None => Ok(manager
            .get_primary_library()
            .map(make_source)
            .into_iter()
            .collect()),
    }
}

fn snippet_json(entry: &LoadedSnippet) -> Value {
    json!({
        "id": entry.snippet.id,
        "library": entry.library,
        "description": entry.snippet.description,
        "command": entry.snippet.command,
        "tags": entry.snippet.tags,
        "folders": entry.snippet.folders,
        "favorite": entry.snippet.favorite,
    })
}

fn snippet_identity_json(entry: &LoadedSnippet) -> Value {
    json!({
        "id": entry.snippet.id,
        "library": entry.library,
        "description": entry.snippet.description,
    })
}

fn library_not_found(name: &str) -> SnipError {
    invalid_params(&format!("Library '{name}' does not exist"))
}

fn invalid_params(message: &str) -> SnipError {
    SnipError::runtime_error("Invalid MCP tool parameters", Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_bounded() {
        assert_eq!(bounded_limit(None).unwrap(), DEFAULT_LIMIT);
        assert!(bounded_limit(Some(0)).is_err());
        assert!(bounded_limit(Some(MAX_LIMIT + 1)).is_err());
    }

    #[test]
    fn arguments_must_be_objects() {
        assert!(parse_arguments::<ListArguments>(Some(&json!(null))).is_err());
        assert!(parse_arguments::<ListArguments>(Some(&json!({"unexpected": true}))).is_err());
    }
}
