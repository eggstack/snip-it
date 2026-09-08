//! **Layer: Domain/Core**
//!
//! Shared selector model for deterministic snippet targeting.
//!
//! Provides [`SnippetSelector`] for non-TUI snippet resolution used by
//! `get`, `run --id`, `clip --id`, `edit --id`, and other exact-targeting
//! commands. The selector never opens a TUI, never executes snippets,
//! and never accesses the clipboard.

use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, Snippet, Snippets};
use std::path::PathBuf;

/// Library scope for snippet resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LibraryScope {
    /// Search the primary library only (default).
    #[default]
    Primary,
    /// Search a specific named library.
    Named(String),
    /// Search all libraries.
    AllLibraries,
}

impl LibraryScope {
    /// Canonical library-scope parsing for CLI `--library` / MCP `library` args.
    ///
    /// `None` selects the primary library; `Some("all")` selects every
    /// registered library; any other name selects that named library.
    /// This is the single definition of `"all"` handling so `snp get`,
    /// exact-mode `run`/`clip`/`edit`, and MCP cannot drift.
    pub fn from_filter_arg(library: Option<&str>) -> Self {
        match library {
            None => LibraryScope::Primary,
            Some("all") => LibraryScope::AllLibraries,
            Some(name) => LibraryScope::Named(name.to_string()),
        }
    }

    /// Owned-`String` counterpart to [`LibraryScope::from_filter_arg`].
    pub fn from_owned_arg(library: Option<String>) -> Self {
        Self::from_filter_arg(library.as_deref())
    }
}

/// Which snippet fields participate in fuzzy search.
///
/// Canonical field contract (Plan 011):
/// - `description` and `command` are always searched;
/// - `tags` are searched when `include_tags` is set (default fuzzy paths
///   enable this; tags are organizational metadata, never credentials);
/// - `output`/notes are searched only when `include_output` is set
///   (`snp list --search-output`, MCP `search_output: true`). Output is
///   bounded via `OutputPresentation::for_scoring()` (512 chars).
/// - `folders`, `favorite`, sync metadata, credentials, and keychain data
///   are never searchable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchFields {
    /// Include space-joined `tags` in the searchable text.
    pub include_tags: bool,
    /// Include bounded `output`/notes summary in the searchable text.
    pub include_output: bool,
}

impl Default for SearchFields {
    fn default() -> Self {
        Self {
            include_tags: true,
            include_output: false,
        }
    }
}

impl SearchFields {
    /// Description + command only (historical minimum).
    pub fn description_and_command() -> Self {
        Self {
            include_tags: false,
            include_output: false,
        }
    }

    /// Default fuzzy contract: description + command + tags.
    pub fn fuzzy_default() -> Self {
        Self::default()
    }

    /// Full text search including bounded output/notes.
    pub fn with_output() -> Self {
        Self {
            include_tags: true,
            include_output: true,
        }
    }
}

/// Build the canonical searchable text for a snippet under `fields`.
///
/// Deleted filtering, fuzzy scoring, and ranking live with the callers;
/// this helper only decides *which fields* participate so CLI (`get --query`,
/// `list --filter`), and MCP (`snippets_search`) cannot drift.
pub fn searchable_text(snippet: &crate::library::Snippet, fields: SearchFields) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(4);
    if !snippet.description.is_empty() {
        parts.push(snippet.description.as_str());
    }
    if !snippet.command.is_empty() {
        parts.push(snippet.command.as_str());
    }
    let tags_joined;
    if fields.include_tags && !snippet.tags.is_empty() {
        tags_joined = snippet.tags.join(" ");
        if !tags_joined.is_empty() {
            parts.push(tags_joined.as_str());
        }
    }
    let output_summary;
    if fields.include_output && !snippet.output.is_empty() {
        output_summary = crate::output::OutputPresentation::new(&snippet.output).for_scoring();
        if !output_summary.is_empty() {
            parts.push(output_summary.as_str());
        }
    }
    parts.join(" ")
}

/// Score non-deleted snippets against `query` using the canonical
/// searchable text and `SkimMatcherV2`.
///
/// Returns `(matched_indices, scores)` where indices refer to positions in
/// `snippets`. An empty query matches every non-deleted snippet with no
/// scores (callers rank those by index order under `Relevance`).
pub fn score_fuzzy_matches(
    snippets: &[crate::library::Snippet],
    query: &str,
    fields: SearchFields,
) -> (Vec<usize>, std::collections::HashMap<usize, i64>) {
    use fuzzy_matcher::FuzzyMatcher;
    use fuzzy_matcher::skim::SkimMatcherV2;

    let matcher = SkimMatcherV2::default();
    let mut indices = Vec::new();
    let mut scores = std::collections::HashMap::new();
    if query.is_empty() {
        for (i, s) in snippets.iter().enumerate() {
            if !s.deleted {
                indices.push(i);
            }
        }
        return (indices, scores);
    }
    for (i, s) in snippets.iter().enumerate() {
        if s.deleted {
            continue;
        }
        let display = searchable_text(s, fields);
        if display.is_empty() {
            continue;
        }
        if let Some(score) = matcher.fuzzy_match(&display, query) {
            scores.insert(i, score);
            indices.push(i);
        }
    }
    (indices, scores)
}

/// Returns `true` when `snippet` carries every tag in `required`
/// (case-insensitive exact tag equality).
pub fn matches_required_tags(snippet: &crate::library::Snippet, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    required.iter().all(|want| {
        let want_lower = want.to_lowercase();
        snippet
            .tags
            .iter()
            .any(|have| have.to_lowercase() == want_lower)
    })
}

/// Resolution policy when a query matches multiple snippets.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ResolutionPolicy {
    /// Return exactly one result; fail if ambiguous or not found.
    #[default]
    Unique,
    /// Return the first result in stable order; never fail for ambiguity.
    First,
    /// Return all matching results.
    All,
}

/// A snippet match result with library context.
#[derive(Debug, Clone)]
pub struct SnippetMatch {
    /// The matched snippet.
    pub snippet: Snippet,
    /// The library file path this snippet belongs to.
    pub library_path: PathBuf,
    /// The library name (filename without .toml).
    pub library_name: String,
    /// The library ID (server-side ID if linked).
    pub library_id: String,
}

/// Identity information for a snippet, used in ambiguity reports.
#[derive(Debug, Clone)]
pub struct SnippetIdentity {
    pub id: String,
    pub description: String,
    pub command: String,
    pub library_name: String,
}

/// The result of a selector resolution attempt.
#[derive(Debug, Clone)]
pub enum SelectionResult {
    /// Exactly one match found.
    One(Box<SnippetMatch>),
    /// Multiple matches found (policy = All).
    Many(Vec<SnippetMatch>),
    /// No match found.
    NotFound,
    /// Multiple matches found but policy is Unique or First is ambiguous.
    Ambiguous(Vec<SnippetIdentity>),
}

/// A deterministic snippet selector for non-TUI resolution.
///
/// All fields are optional except `resolution` and `library`. At least one
/// of `id`, `description_exact`, `command_exact`, or `query` must be set.
#[derive(Debug, Clone)]
pub struct SnippetSelector {
    /// Match by exact snippet UUID.
    pub id: Option<String>,
    /// Match by exact description (case-insensitive).
    pub description_exact: Option<String>,
    /// Match by exact command text (case-insensitive).
    pub command_exact: Option<String>,
    /// Fuzzy query match (uses existing ranking).
    pub query: Option<String>,
    /// Library scope.
    pub library: LibraryScope,
    /// Resolution policy for multiple matches.
    pub resolution: ResolutionPolicy,
}

impl SnippetSelector {
    /// Create a new selector with the given resolution policy.
    pub fn new(resolution: ResolutionPolicy) -> Self {
        Self {
            id: None,
            description_exact: None,
            command_exact: None,
            query: None,
            library: LibraryScope::Primary,
            resolution,
        }
    }

    /// Set ID matching.
    pub fn with_id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    /// Set exact description matching.
    pub fn with_description_exact(mut self, desc: String) -> Self {
        self.description_exact = Some(desc);
        self
    }

    /// Set exact command matching.
    pub fn with_command_exact(mut self, cmd: String) -> Self {
        self.command_exact = Some(cmd);
        self
    }

    /// Set fuzzy query matching.
    pub fn with_query(mut self, query: String) -> Self {
        self.query = Some(query);
        self
    }

    /// Set library scope.
    pub fn with_library(mut self, library: LibraryScope) -> Self {
        self.library = library;
        self
    }

    /// Validate that the selector has at least one targeting field set.
    fn validate(&self) -> SnipResult<()> {
        if self.id.is_none()
            && self.description_exact.is_none()
            && self.command_exact.is_none()
            && self.query.is_none()
        {
            return Err(SnipError::runtime_error(
                "No selector specified",
                Some("Provide --id, --description-exact, --command-exact, or --query"),
            ));
        }
        Ok(())
    }

    /// Resolve the selector against the given snippets and library context.
    ///
    /// This is the core resolution logic. It applies selectors in priority order:
    /// 1. ID (exact, case-sensitive UUID match)
    /// 2. Exact description (case-insensitive)
    /// 3. Exact command (case-insensitive)
    /// 4. Query (fuzzy over canonical [`searchable_text`] with
    ///    [`SearchFields::fuzzy_default`] — description, command, tags —
    ///    ranked by `Relevance`)
    ///
    /// Deleted snippets are excluded from every mode. Within one library the
    /// file order is preserved for exact matches and `Relevance` ranking
    /// decides fuzzy order; cross-library aggregation adds the deterministic
    /// library → description → ID tie-break in [`finish_aggregate`].
    pub fn resolve(
        &self,
        snippets: &Snippets,
        lib_path: &std::path::Path,
        lib_name: &str,
        lib_id: &str,
    ) -> SnipResult<SelectionResult> {
        self.validate()?;

        let active: Vec<&Snippet> = snippets.snippets.iter().filter(|s| !s.deleted).collect();

        // 1. ID lookup (exact)
        if let Some(ref id) = self.id {
            let matches: Vec<&Snippet> = active
                .iter()
                .filter(|s| s.id == id.as_str())
                .copied()
                .collect();
            return match matches.len() {
                0 => Ok(SelectionResult::NotFound),
                1 => Ok(SelectionResult::One(Box::new(SnippetMatch {
                    snippet: matches[0].clone(),
                    library_path: lib_path.to_path_buf(),
                    library_name: lib_name.to_string(),
                    library_id: lib_id.to_string(),
                }))),
                _ => {
                    // Multiple snippets with same ID (shouldn't happen with dedup, but be safe)
                    let identities: Vec<SnippetIdentity> = matches
                        .iter()
                        .map(|s| SnippetIdentity {
                            id: s.id.clone(),
                            description: s.description.clone(),
                            command: s.command.clone(),
                            library_name: lib_name.to_string(),
                        })
                        .collect();
                    Ok(SelectionResult::Ambiguous(identities))
                }
            };
        }

        // 2. Exact description (case-insensitive)
        if let Some(ref desc) = self.description_exact {
            let desc_lower = desc.to_lowercase();
            let matches: Vec<&Snippet> = active
                .iter()
                .filter(|s| s.description.to_lowercase() == desc_lower)
                .copied()
                .collect();
            return self.resolve_matches(matches, lib_path, lib_name, lib_id);
        }

        // 3. Exact command (case-insensitive)
        if let Some(ref cmd) = self.command_exact {
            let cmd_lower = cmd.to_lowercase();
            let matches: Vec<&Snippet> = active
                .iter()
                .filter(|s| s.command.to_lowercase() == cmd_lower)
                .copied()
                .collect();
            return self.resolve_matches(matches, lib_path, lib_name, lib_id);
        }

        // 4. Query (fuzzy match over canonical searchable text)
        if let Some(ref query) = self.query {
            let (active_indices, fuzzy_scores) =
                score_fuzzy_matches(&snippets.snippets, query, SearchFields::fuzzy_default());

            if active_indices.is_empty() {
                return Ok(SelectionResult::NotFound);
            }

            use crate::sort::{SnippetSort, SortOptions, rank_snippets};

            let opts = SortOptions {
                mode: SnippetSort::Relevance,
                ..Default::default()
            };
            let ranked = rank_snippets(
                &active_indices,
                &snippets.snippets,
                Some(&fuzzy_scores),
                None,
                &opts,
            );

            let matches: Vec<&Snippet> = ranked
                .iter()
                .filter(|idx| {
                    // Empty query yields no scores; every candidate is a match.
                    // Non-empty queries only keep fuzzy hits.
                    query.is_empty() || fuzzy_scores.contains_key(idx)
                })
                .map(|idx| &snippets.snippets[*idx])
                .collect();

            return self.resolve_matches(matches, lib_path, lib_name, lib_id);
        }

        // Should not reach here due to validate() check
        Ok(SelectionResult::NotFound)
    }

    /// Apply the resolution policy to a set of matched snippets.
    fn resolve_matches(
        &self,
        matches: Vec<&Snippet>,
        lib_path: &std::path::Path,
        lib_name: &str,
        lib_id: &str,
    ) -> SnipResult<SelectionResult> {
        match self.resolution {
            ResolutionPolicy::All => {
                let results: Vec<SnippetMatch> = matches
                    .into_iter()
                    .map(|s| SnippetMatch {
                        snippet: s.clone(),
                        library_path: lib_path.to_path_buf(),
                        library_name: lib_name.to_string(),
                        library_id: lib_id.to_string(),
                    })
                    .collect();
                if results.is_empty() {
                    Ok(SelectionResult::NotFound)
                } else {
                    Ok(SelectionResult::Many(results))
                }
            }
            ResolutionPolicy::First => match matches.first() {
                Some(s) => Ok(SelectionResult::One(Box::new(SnippetMatch {
                    snippet: (*s).clone(),
                    library_path: lib_path.to_path_buf(),
                    library_name: lib_name.to_string(),
                    library_id: lib_id.to_string(),
                }))),
                None => Ok(SelectionResult::NotFound),
            },
            ResolutionPolicy::Unique => match matches.len() {
                0 => Ok(SelectionResult::NotFound),
                1 => Ok(SelectionResult::One(Box::new(SnippetMatch {
                    snippet: matches[0].clone(),
                    library_path: lib_path.to_path_buf(),
                    library_name: lib_name.to_string(),
                    library_id: lib_id.to_string(),
                }))),
                _ => {
                    let identities: Vec<SnippetIdentity> = matches
                        .iter()
                        .map(|s| SnippetIdentity {
                            id: s.id.clone(),
                            description: s.description.clone(),
                            command: s.command.clone(),
                            library_name: lib_name.to_string(),
                        })
                        .collect();
                    Ok(SelectionResult::Ambiguous(identities))
                }
            },
        }
    }
}

/// Sort snippet matches deterministically: library name → description → snippet ID.
pub fn sort_matches(matches: &mut [SnippetMatch]) {
    matches.sort_by(|a, b| {
        a.library_name
            .to_lowercase()
            .cmp(&b.library_name.to_lowercase())
            .then_with(|| {
                a.snippet
                    .description
                    .to_lowercase()
                    .cmp(&b.snippet.description.to_lowercase())
            })
            .then_with(|| a.snippet.id.cmp(&b.snippet.id))
    });
}

/// Apply the resolution policy to a cross-library aggregate.
///
/// Single canonical definition of `all`-scope ambiguity behavior:
/// deterministic [`sort_matches`] order first, then `All` → `Many`,
/// `First` → first `One`, `Unique` → `Ambiguous` when more than one.
/// `NotFound` when empty. Both [`resolve_selector`] and
/// [`resolve_selector_readonly`] (and MCP `get`) share this path.
pub fn finish_aggregate(
    mut all_matches: Vec<SnippetMatch>,
    resolution: &ResolutionPolicy,
) -> SnipResult<SelectionResult> {
    sort_matches(&mut all_matches);
    match all_matches.len() {
        0 => Ok(SelectionResult::NotFound),
        1 => {
            let Some(m) = all_matches.into_iter().next() else {
                return Ok(SelectionResult::NotFound);
            };
            Ok(SelectionResult::One(Box::new(m)))
        }
        _ => match resolution {
            ResolutionPolicy::All => Ok(SelectionResult::Many(all_matches)),
            ResolutionPolicy::First => {
                let Some(m) = all_matches.into_iter().next() else {
                    return Ok(SelectionResult::NotFound);
                };
                Ok(SelectionResult::One(Box::new(m)))
            }
            ResolutionPolicy::Unique => {
                let identities: Vec<SnippetIdentity> = all_matches
                    .iter()
                    .map(|m| SnippetIdentity {
                        id: m.snippet.id.clone(),
                        description: m.snippet.description.clone(),
                        command: m.snippet.command.clone(),
                        library_name: m.library_name.clone(),
                    })
                    .collect();
                Ok(SelectionResult::Ambiguous(identities))
            }
        },
    }
}

/// Build an exact-target selector without resolving it.
///
/// Canonical constructor for `run`/`clip`/`edit` exact paths and `snp get`
/// exact fields. Library `"all"` maps to [`LibraryScope::AllLibraries`] via
/// [`LibraryScope::from_owned_arg`]; ID is case-sensitive, description and
/// command are case-insensitive (see [`SnippetSelector::resolve`]).
pub fn exact_selector(
    library: Option<String>,
    id: Option<String>,
    description_exact: Option<String>,
    command_exact: Option<String>,
) -> SnippetSelector {
    let mut selector = SnippetSelector::new(ResolutionPolicy::Unique)
        .with_library(LibraryScope::from_owned_arg(library));
    if let Some(id) = id {
        selector = selector.with_id(id);
    }
    if let Some(desc) = description_exact {
        selector = selector.with_description_exact(desc);
    }
    if let Some(cmd) = command_exact {
        selector = selector.with_command_exact(cmd);
    }
    selector
}

/// Construct and resolve an exact-target selector from CLI arguments.
///
/// This is the single canonical path for exact `Run`, `Clip`, and `Edit`
/// targeting. It builds a `SnippetSelector` with `ResolutionPolicy::Unique`,
/// applies the provided filters, and calls [`resolve_selector`].
pub fn resolve_exact_target(
    library: Option<String>,
    id: Option<String>,
    description_exact: Option<String>,
    command_exact: Option<String>,
) -> SnipResult<SelectionResult> {
    let selector = exact_selector(library, id, description_exact, command_exact);
    resolve_selector(&selector)
}

/// Resolve a selector without triggering legacy migration or file creation.
///
/// Side-effect-free counterpart to [`resolve_selector`] for deterministic
/// read paths (`snp get`, MCP). It consumes the canonical
/// [`crate::library::readonly_library_sources`] resolver, so legacy
/// single-file handling, primary resolution, and path construction cannot
/// drift from the library layer. A `Primary` scope with no visible source
/// preserves the historical "No primary library" error; an `all` scope with
/// no visible libraries resolves to [`SelectionResult::NotFound`].
pub fn resolve_selector_readonly(selector: &SnippetSelector) -> SnipResult<SelectionResult> {
    let scope_arg: Option<String> = match &selector.library {
        LibraryScope::Primary => None,
        LibraryScope::Named(name) => Some(name.clone()),
        LibraryScope::AllLibraries => Some("all".to_string()),
    };
    let sources = crate::library::readonly_library_sources(scope_arg.as_deref())?;

    if sources.is_empty() {
        match &selector.library {
            LibraryScope::Primary => {
                return Err(SnipError::runtime_error(
                    "No primary library",
                    Some("Create a library with 'snp library create <name>'"),
                ));
            }
            LibraryScope::AllLibraries => return Ok(SelectionResult::NotFound),
            LibraryScope::Named(_) => {
                // `readonly_library_sources` already errors for unknown names;
                // an empty result here means no visible libraries.
                return Ok(SelectionResult::NotFound);
            }
        }
    }

    if sources.len() == 1 {
        let source = &sources[0];
        let snippets = crate::library::load_library(&source.path)?;
        return selector.resolve(&snippets, &source.path, &source.name, &source.library_id);
    }

    let mut all_matches: Vec<SnippetMatch> = Vec::new();
    // Collect with `All` policy per library so a multi-match inside one
    // library is not dropped as `Ambiguous` before the cross-library
    // `finish_aggregate` can apply the caller's real policy.
    let collector = SnippetSelector {
        resolution: ResolutionPolicy::All,
        ..selector.clone()
    };
    for source in &sources {
        let snippets = crate::library::load_library(&source.path)?;
        let result =
            collector.resolve(&snippets, &source.path, &source.name, &source.library_id)?;
        match result {
            SelectionResult::One(m) => all_matches.push(*m),
            SelectionResult::Many(ms) => all_matches.extend(ms),
            _ => {}
        }
    }

    finish_aggregate(all_matches, &selector.resolution)
}

/// Resolve a selector across potentially multiple libraries.
///
/// This is the top-level entry point for non-TUI snippet resolution.
/// It loads the appropriate libraries based on `LibraryScope` and
/// applies the selector to each.
pub fn resolve_selector(selector: &SnippetSelector) -> SnipResult<SelectionResult> {
    let mut mgr = LibraryManager::new()?;
    mgr.ensure_library_mode()?;

    match &selector.library {
        LibraryScope::Primary => {
            let primary = mgr.get_primary_library().ok_or_else(|| {
                SnipError::runtime_error(
                    "No primary library",
                    Some("Create a library with 'snp library create <name>'"),
                )
            })?;
            let path = mgr
                .get_libraries_dir()
                .join(format!("{}.toml", primary.filename));
            let snippets = crate::library::load_library(&path)?;
            let lib_id = primary.library_id.clone();
            selector.resolve(&snippets, &path, &primary.filename, &lib_id)
        }
        LibraryScope::Named(name) => {
            let lib = mgr
                .get_library_by_filename(name)
                .ok_or_else(|| crate::library::library_not_found(name))?;
            let path = mgr
                .get_libraries_dir()
                .join(format!("{}.toml", lib.filename));
            let snippets = crate::library::load_library(&path)?;
            let lib_id = lib.library_id.clone();
            selector.resolve(&snippets, &path, &lib.filename, &lib_id)
        }
        LibraryScope::AllLibraries => {
            let mut all_matches: Vec<SnippetMatch> = Vec::new();
            // Same `All`-for-collection rationale as the readonly path: a
            // per-library `Ambiguous` under `Unique` must still contribute
            // its candidates to the cross-library verdict.
            let collector = SnippetSelector {
                resolution: ResolutionPolicy::All,
                ..selector.clone()
            };
            for lib in mgr.list_libraries() {
                let path = mgr
                    .get_libraries_dir()
                    .join(format!("{}.toml", lib.filename));
                let snippets = crate::library::load_library(&path)?;
                let lib_id = lib.library_id.clone();
                let result = collector.resolve(&snippets, &path, &lib.filename, &lib_id)?;
                match result {
                    SelectionResult::One(m) => all_matches.push(*m),
                    SelectionResult::Many(ms) => all_matches.extend(ms),
                    _ => {}
                }
            }

            finish_aggregate(all_matches, &selector.resolution)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Snippet;

    fn make_snippet(id: &str, desc: &str, cmd: &str) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            ..Default::default()
        }
    }

    fn make_snippets() -> Snippets {
        Snippets {
            snippets: vec![
                make_snippet("aaa-111", "git commit", "git commit -m \"msg\""),
                make_snippet("bbb-222", "git push", "git push origin main"),
                make_snippet("ccc-333", "list files", "ls -la"),
            ],
            folders: vec![],
        }
    }

    #[test]
    fn test_resolve_by_id() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_id("bbb-222".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.snippet.id, "bbb-222");
                assert_eq!(m.snippet.description, "git push");
            }
            _ => panic!("Expected One match"),
        }
    }

    #[test]
    fn test_resolve_by_id_not_found() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_id("zzz-999".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        assert!(matches!(result, SelectionResult::NotFound));
    }

    #[test]
    fn test_resolve_by_description_exact() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_description_exact("Git Commit".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.snippet.description, "git commit");
            }
            _ => panic!("Expected One match"),
        }
    }

    #[test]
    fn test_resolve_by_description_ambiguous() {
        let mut snippets = make_snippets();
        snippets
            .snippets
            .push(make_snippet("ddd-444", "git commit", "git commit --amend"));
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_description_exact("git commit".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        assert!(matches!(result, SelectionResult::Ambiguous(_)));
    }

    #[test]
    fn test_resolve_by_description_first() {
        let mut snippets = make_snippets();
        snippets
            .snippets
            .push(make_snippet("ddd-444", "git commit", "git commit --amend"));
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::First)
            .with_description_exact("git commit".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        assert!(matches!(result, SelectionResult::One(_)));
    }

    #[test]
    fn test_resolve_by_command_exact() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_command_exact("ls -la".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.snippet.command, "ls -la");
            }
            _ => panic!("Expected One match"),
        }
    }

    #[test]
    fn test_resolve_by_query() {
        use fuzzy_matcher::FuzzyMatcher;
        use fuzzy_matcher::skim::SkimMatcherV2;

        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");

        // Debug: verify fuzzy matcher works directly
        let matcher = SkimMatcherV2::default();
        let s1 = "git commit";
        let s2 = "git push";
        let score1 = matcher.fuzzy_match(s1, "git");
        let score2 = matcher.fuzzy_match(s2, "git");
        eprintln!(
            "Direct fuzzy: 'git commit' vs 'git' = {score1:?}, 'git push' vs 'git' = {score2:?}"
        );

        // Unique resolution with ambiguous query should return Ambiguous
        let selector = SnippetSelector::new(ResolutionPolicy::Unique).with_query("git".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert!(m.snippet.command.contains("git") || m.snippet.description.contains("git"));
            }
            SelectionResult::Many(_) => {} // All policy returns many
            SelectionResult::Ambiguous(_) => {} // Unique with multiple matches is ambiguous
            SelectionResult::NotFound => panic!("Expected at least one match"),
        }

        // First resolution should return first match
        let selector_first =
            SnippetSelector::new(ResolutionPolicy::First).with_query("git".to_string());
        let result_first = selector_first
            .resolve(&snippets, &path, "test", "")
            .unwrap();
        assert!(matches!(result_first, SelectionResult::One(_)));

        // All resolution should return all matches
        let selector_all =
            SnippetSelector::new(ResolutionPolicy::All).with_query("git".to_string());
        let result_all = selector_all.resolve(&snippets, &path, "test", "").unwrap();
        assert!(matches!(result_all, SelectionResult::Many(_)));
    }

    #[test]
    fn test_resolve_excludes_deleted() {
        let mut snippets = make_snippets();
        snippets.snippets[0].deleted = true;
        let path = PathBuf::from("/tmp/test.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_id("aaa-111".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        assert!(matches!(result, SelectionResult::NotFound));
    }

    #[test]
    fn test_validate_no_fields() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique);
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let result = selector.resolve(&snippets, &path, "test", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_snippet_match_includes_library_context() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/work.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_id("aaa-111".to_string());

        let result = selector
            .resolve(&snippets, &path, "work", "lib-123")
            .unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.library_name, "work");
                assert_eq!(m.library_id, "lib-123");
                assert_eq!(m.library_path, path);
            }
            _ => panic!("Expected One match"),
        }
    }

    #[test]
    fn test_all_policy_returns_many() {
        let mut snippets = make_snippets();
        snippets
            .snippets
            .push(make_snippet("ddd-444", "git commit", "git commit --amend"));
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::All)
            .with_description_exact("git commit".to_string());

        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::Many(matches) => assert_eq!(matches.len(), 2),
            _ => panic!("Expected Many matches"),
        }
    }

    // ── resolve_exact_target construction tests (§4.4 acceptance) ────

    #[test]
    fn test_resolve_exact_target_builds_unique_policy_selector() {
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_library(LibraryScope::Primary);
        assert_eq!(selector.resolution, ResolutionPolicy::Unique);
        assert_eq!(selector.library, LibraryScope::Primary);
        assert!(selector.id.is_none());
        assert!(selector.description_exact.is_none());
        assert!(selector.command_exact.is_none());
    }

    #[test]
    fn test_resolve_exact_target_id_field() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_id("test-id".to_string())
            .with_library(LibraryScope::Primary);
        assert_eq!(selector.id.as_deref(), Some("test-id"));
        assert!(selector.description_exact.is_none());
        assert!(selector.command_exact.is_none());
    }

    #[test]
    fn test_resolve_exact_target_description_field() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_description_exact("my desc".to_string())
            .with_library(LibraryScope::Primary);
        assert!(selector.id.is_none());
        assert_eq!(selector.description_exact.as_deref(), Some("my desc"));
        assert!(selector.command_exact.is_none());
    }

    #[test]
    fn test_resolve_exact_target_command_field() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_command_exact("echo hello".to_string())
            .with_library(LibraryScope::Primary);
        assert!(selector.id.is_none());
        assert!(selector.description_exact.is_none());
        assert_eq!(selector.command_exact.as_deref(), Some("echo hello"));
    }

    #[test]
    fn test_resolve_exact_target_named_library_scope() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_library(LibraryScope::Named("work".to_string()));
        assert_eq!(selector.library, LibraryScope::Named("work".to_string()));
    }

    #[test]
    fn test_resolve_exact_target_all_libraries_scope() {
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_library(LibraryScope::AllLibraries);
        assert_eq!(selector.library, LibraryScope::AllLibraries);
    }

    #[test]
    fn test_resolve_exact_target_all_fields() {
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_id("id-1".to_string())
            .with_description_exact("desc-1".to_string())
            .with_command_exact("cmd-1".to_string())
            .with_library(LibraryScope::Named("lib".to_string()));
        assert_eq!(selector.id.as_deref(), Some("id-1"));
        assert_eq!(selector.description_exact.as_deref(), Some("desc-1"));
        assert_eq!(selector.command_exact.as_deref(), Some("cmd-1"));
        assert_eq!(selector.library, LibraryScope::Named("lib".to_string()));
    }

    #[test]
    fn test_resolve_exact_target_id_overrides_description_and_command() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_id("aaa-111".to_string())
            .with_description_exact("git push".to_string())
            .with_command_exact("ls -la".to_string());
        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.snippet.id, "aaa-111");
                assert_eq!(m.snippet.description, "git commit");
            }
            _ => panic!("Expected One match by ID, ignoring description/command"),
        }
    }

    #[test]
    fn test_resolve_exact_target_description_overrides_command() {
        let snippets = make_snippets();
        let path = PathBuf::from("/tmp/test.toml");
        let selector = SnippetSelector::new(ResolutionPolicy::Unique)
            .with_description_exact("git commit".to_string())
            .with_command_exact("ls -la".to_string());
        let result = selector.resolve(&snippets, &path, "test", "").unwrap();
        match result {
            SelectionResult::One(m) => {
                assert_eq!(m.snippet.description, "git commit");
            }
            _ => panic!("Expected One match by description, ignoring command"),
        }
    }

    // ── Plan 011: canonical scope / searchable-text / aggregate ────

    #[test]
    fn test_library_scope_from_filter_arg() {
        assert_eq!(LibraryScope::from_filter_arg(None), LibraryScope::Primary);
        assert_eq!(
            LibraryScope::from_filter_arg(Some("all")),
            LibraryScope::AllLibraries
        );
        assert_eq!(
            LibraryScope::from_filter_arg(Some("work")),
            LibraryScope::Named("work".to_string())
        );
        assert_eq!(
            LibraryScope::from_owned_arg(Some("all".to_string())),
            LibraryScope::AllLibraries
        );
    }

    #[test]
    fn test_exact_selector_maps_all_to_all_libraries() {
        let selector = exact_selector(Some("all".to_string()), None, None, None);
        assert_eq!(selector.library, LibraryScope::AllLibraries);
        assert_eq!(selector.resolution, ResolutionPolicy::Unique);
    }

    #[test]
    fn test_searchable_text_field_contract() {
        let mut snippet = make_snippet("id-1", "Deploy service", "kubectl apply");
        snippet.tags = vec!["deploy".to_string(), "k8s".to_string()];
        snippet.output = "sample output note".to_string();

        let base = searchable_text(&snippet, SearchFields::description_and_command());
        assert_eq!(base, "Deploy service kubectl apply");

        let fuzzy = searchable_text(&snippet, SearchFields::fuzzy_default());
        assert_eq!(fuzzy, "Deploy service kubectl apply deploy k8s");

        let full = searchable_text(&snippet, SearchFields::with_output());
        assert!(full.contains("sample output note"));

        // Folders, favorite, and sync metadata never participate.
        snippet.folders = vec!["ops".to_string()];
        snippet.favorite = true;
        let again = searchable_text(&snippet, SearchFields::with_output());
        assert!(!again.contains("ops"));
    }

    #[test]
    fn test_score_fuzzy_matches_empty_query_returns_all_live() {
        let mut snippets = make_snippets();
        snippets.snippets[0].deleted = true;
        let (indices, scores) =
            score_fuzzy_matches(&snippets.snippets, "", SearchFields::fuzzy_default());
        assert_eq!(indices, vec![1, 2]);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_score_fuzzy_matches_tag_only_hit() {
        let mut snippets = make_snippets();
        snippets.snippets[0].tags = vec!["uniquetag123".to_string()];
        let (indices, _) = score_fuzzy_matches(
            &snippets.snippets,
            "uniquetag123",
            SearchFields::fuzzy_default(),
        );
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn test_score_fuzzy_matches_output_opt_in() {
        let mut snippets = make_snippets();
        snippets.snippets[0].output = "uniqoutput456 note".to_string();
        let (without, _) = score_fuzzy_matches(
            &snippets.snippets,
            "uniqoutput456",
            SearchFields::fuzzy_default(),
        );
        assert!(without.is_empty());
        let (with, _) = score_fuzzy_matches(
            &snippets.snippets,
            "uniqoutput456",
            SearchFields::with_output(),
        );
        assert_eq!(with, vec![0]);
    }

    #[test]
    fn test_matches_required_tags_case_insensitive_all() {
        let mut snippet = make_snippet("id-1", "desc", "cmd");
        snippet.tags = vec!["Deploy".to_string(), "K8s".to_string()];
        assert!(matches_required_tags(&snippet, &[]));
        assert!(matches_required_tags(&snippet, &["deploy".to_string()]));
        assert!(matches_required_tags(
            &snippet,
            &["DEPLOY".to_string(), "k8s".to_string()]
        ));
        assert!(!matches_required_tags(&snippet, &["missing".to_string()]));
    }

    #[test]
    fn test_finish_aggregate_deterministic_order_and_policies() {
        let path = PathBuf::from("/tmp/test.toml");
        let mk = |id: &str, desc: &str, lib: &str| SnippetMatch {
            snippet: make_snippet(id, desc, "cmd"),
            library_path: path.clone(),
            library_name: lib.to_string(),
            library_id: String::new(),
        };
        let matches = vec![
            mk("b-2", "Beta", "work"),
            mk("a-1", "alpha", "personal"),
            mk("b-1", "Beta", "work"),
        ];
        match finish_aggregate(matches, &ResolutionPolicy::All).unwrap() {
            SelectionResult::Many(ms) => {
                let ids: Vec<&str> = ms.iter().map(|m| m.snippet.id.as_str()).collect();
                assert_eq!(ids, vec!["a-1", "b-1", "b-2"]);
            }
            _ => panic!("expected Many"),
        }
        let matches = vec![mk("b-2", "Beta", "work"), mk("a-1", "alpha", "personal")];
        match finish_aggregate(matches, &ResolutionPolicy::First).unwrap() {
            SelectionResult::One(m) => assert_eq!(m.snippet.id, "a-1"),
            _ => panic!("expected One"),
        }
        let matches = vec![mk("b-2", "Beta", "work"), mk("a-1", "alpha", "personal")];
        match finish_aggregate(matches, &ResolutionPolicy::Unique).unwrap() {
            SelectionResult::Ambiguous(ids) => assert_eq!(ids.len(), 2),
            _ => panic!("expected Ambiguous"),
        }
    }

    #[test]
    fn test_query_includes_tags_via_canonical_helper() {
        let mut snippets = make_snippets();
        snippets.snippets[0].description = "Unrelated".to_string();
        snippets.snippets[0].command = "echo unrelated".to_string();
        snippets.snippets[0].tags = vec!["uniquetag123".to_string()];
        let path = PathBuf::from("/tmp/test.toml");
        let selector =
            SnippetSelector::new(ResolutionPolicy::Unique).with_query("uniquetag123".to_string());
        match selector.resolve(&snippets, &path, "test", "").unwrap() {
            SelectionResult::One(m) => assert_eq!(m.snippet.id, "aaa-111"),
            _ => panic!("tag text should participate in fuzzy query"),
        }
    }
}
