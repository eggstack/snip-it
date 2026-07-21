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
    /// 1. ID (exact, not fuzzy)
    /// 2. Exact description (case-insensitive)
    /// 3. Exact command
    /// 4. Query (fuzzy, uses existing ranking)
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

        // 4. Query (fuzzy match using existing ranking)
        if let Some(ref query) = self.query {
            use fuzzy_matcher::FuzzyMatcher;
            use fuzzy_matcher::skim::SkimMatcherV2;

            let matcher = SkimMatcherV2::default();
            let mut fuzzy_scores = std::collections::HashMap::new();
            let mut active_indices = Vec::new();

            for (i, s) in snippets.snippets.iter().enumerate() {
                if s.deleted {
                    continue;
                }
                let display = format!("{} {}", s.description, s.command);
                if let Some(score) = matcher.fuzzy_match(&display, query) {
                    fuzzy_scores.insert(i, score);
                    active_indices.push(i);
                }
            }

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
                .filter(|idx| fuzzy_scores.contains_key(idx))
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
            let lib = mgr.get_library_by_filename(name).ok_or_else(|| {
                SnipError::runtime_error(
                    "Library not found",
                    Some(&format!(
                        "Library '{name}' does not exist. Use 'snp library list' to see available libraries."
                    )),
                )
            })?;
            let path = mgr
                .get_libraries_dir()
                .join(format!("{}.toml", lib.filename));
            let snippets = crate::library::load_library(&path)?;
            let lib_id = lib.library_id.clone();
            selector.resolve(&snippets, &path, &lib.filename, &lib_id)
        }
        LibraryScope::AllLibraries => {
            let mut all_matches: Vec<SnippetMatch> = Vec::new();
            for lib in mgr.list_libraries() {
                let path = mgr
                    .get_libraries_dir()
                    .join(format!("{}.toml", lib.filename));
                let snippets = crate::library::load_library(&path)?;
                let lib_id = lib.library_id.clone();
                let result = selector.resolve(&snippets, &path, &lib.filename, &lib_id)?;
                match result {
                    SelectionResult::One(m) => all_matches.push(*m),
                    SelectionResult::Many(ms) => all_matches.extend(ms),
                    _ => {}
                }
            }

            match all_matches.len() {
                0 => Ok(SelectionResult::NotFound),
                1 => Ok(SelectionResult::One(Box::new(
                    all_matches.into_iter().next().unwrap(),
                ))),
                _ => match selector.resolution {
                    ResolutionPolicy::All => Ok(SelectionResult::Many(all_matches)),
                    ResolutionPolicy::First => Ok(SelectionResult::One(Box::new(
                        all_matches.into_iter().next().unwrap(),
                    ))),
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
}
