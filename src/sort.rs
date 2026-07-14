//! Shared sort and ranking model for snippet selection.
//!
//! Provides deterministic, stable sorting of snippets with support for
//! multiple sort modes, favorites-first grouping, and optional fuzzy-relevance
//! scores. The tie-break chain is:
//!
//! 1. Requested primary key ([`SnippetSort`] variant)
//! 2. Favorites-first grouping (orthogonal modifier)
//! 3. Fuzzy relevance where meaningful (for [`SnippetSort::Relevance`])
//! 4. Normalized description (case-insensitive)
//! 5. Original source order (index ascending)

use crate::usage::UsageData;

/// Sort mode for snippet ordering.
///
/// Each variant defines a primary sort key. Within equal primary keys, a
/// deterministic tie-break chain ensures stable, reproducible ordering.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    Default,
    PartialOrd,
    Ord,
    clap::ValueEnum,
)]
#[value(rename_all = "kebab-case")]
pub enum SnippetSort {
    /// Sort by fuzzy relevance score descending (highest first).
    #[default]
    Relevance,
    /// Sort by most recently updated, then most recently created.
    Recent,
    /// Sort by most recently used.
    LastUsed,
    /// Sort by total use count descending.
    MostUsed,
    /// Sort by description alphabetically (case-insensitive).
    Description,
    /// Sort by command alphabetically (case-insensitive).
    Command,
}

/// Options that control how [`rank_snippets`] sorts and groups results.
#[derive(Debug, Clone, Default)]
pub struct SortOptions {
    /// Primary sort mode.
    pub mode: SnippetSort,
    /// When `true`, favorited snippets are grouped before non-favorited
    /// snippets. Within each group the selected sort mode applies unchanged.
    pub favorites_first: bool,
}

/// A snippet index paired with the metadata needed for stable sorting.
///
/// Used internally by [`rank_snippets`] to compute sort keys without
/// rearranging the original snippet data.
#[derive(Debug, Clone)]
struct RankedSnippet {
    /// Index into the caller's snippet slice.
    index: usize,
    /// Normalized (lowercased) description for deterministic alphabetical
    /// tie-breaking.
    desc_lower: String,
    /// Normalized (lowercased) command for deterministic alphabetical
    /// tie-breaking.
    cmd_lower: String,
    /// `true` if the snippet is a favorite.
    favorite: bool,
    /// `updated_at` timestamp (epoch seconds).
    updated_at: i64,
    /// `created_at` timestamp (epoch seconds).
    created_at: i64,
    /// Total use count from [`UsageData`].
    use_count: u64,
    /// Most-recent use timestamp from [`UsageData`], if any.
    last_used_at: Option<i64>,
    /// Optional fuzzy relevance score. Higher values indicate better matches.
    /// `None` means no score was computed (treated as lowest relevance).
    fuzzy_score: Option<i64>,
}

/// Rank and sort a collection of snippet indices according to `opts`.
///
/// # Arguments
///
/// * `indices` – slice of snippet indices to sort (e.g. `0..snippets.len()`).
///   Must not contain duplicates.
/// * `snippets` – the source snippet array; `indices` are indices into this.
/// * `fuzzy_scores` – optional parallel map of `index → i64` fuzzy scores.
///   Only meaningful for [`SnippetSort::Relevance`]. Missing entries are
///   treated as `None`.
/// * `usage` – optional parallel slice of [`UsageData`], one per snippet.
///   Missing entries default to zero uses / no last-used timestamp.
/// * `opts` – sort mode and grouping options.
///
/// # Returns
///
/// A `Vec<usize>` of snippet indices in the sorted order. Ties are always
/// broken deterministically by original index ascending, guaranteeing
/// stable output for identical inputs.
pub fn rank_snippets(
    indices: &[usize],
    snippets: &[crate::library::Snippet],
    fuzzy_scores: Option<&std::collections::HashMap<usize, i64>>,
    usage: Option<&[UsageData]>,
    opts: &SortOptions,
) -> Vec<usize> {
    let ranked: Vec<RankedSnippet> = indices
        .iter()
        .map(|&idx| {
            let s = &snippets[idx];
            let u = usage.and_then(|u| u.get(idx)).cloned().unwrap_or_default();
            let fuzzy_score = fuzzy_scores.and_then(|m| m.get(&idx).copied());
            RankedSnippet {
                index: idx,
                desc_lower: s.description.to_lowercase(),
                cmd_lower: s.command.to_lowercase(),
                favorite: s.favorite,
                updated_at: s.updated_at,
                created_at: s.created_at,
                use_count: u.use_count,
                last_used_at: u.last_used_at,
                fuzzy_score,
            }
        })
        .collect();

    let mut sorted = ranked;
    sorted.sort_by(|a, b| {
        // Favorites-first is an orthogonal grouping: compare favorite status
        // before the primary key so favorites land in a contiguous block.
        if opts.favorites_first {
            match a.favorite.cmp(&b.favorite) {
                std::cmp::Ordering::Equal => {}
                other => return other.reverse(), // true before false
            }
        }

        let primary = match opts.mode {
            SnippetSort::Relevance => {
                // Higher score is better; None scores sort after all Some.
                match (&a.fuzzy_score, &b.fuzzy_score) {
                    (Some(sa), Some(sb)) => sb.cmp(sa),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            }
            SnippetSort::Recent => match b.updated_at.cmp(&a.updated_at) {
                std::cmp::Ordering::Equal => b.created_at.cmp(&a.created_at),
                other => other,
            },
            SnippetSort::LastUsed => {
                // Higher last_used_at first; None sorts after Some.
                match (&a.last_used_at, &b.last_used_at) {
                    (Some(au), Some(bu)) => bu.cmp(au),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            }
            SnippetSort::MostUsed => match b.use_count.cmp(&a.use_count) {
                std::cmp::Ordering::Equal => {
                    // Higher last_used_at first; None sorts after Some.
                    match (&a.last_used_at, &b.last_used_at) {
                        (Some(au), Some(bu)) => bu.cmp(au),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
                other => other,
            },
            SnippetSort::Description => a.desc_lower.cmp(&b.desc_lower),
            SnippetSort::Command => a.cmd_lower.cmp(&b.cmd_lower),
        };

        // Tie-break chain after primary key:
        // 3. Fuzzy relevance (only when not the primary sort)
        if primary == std::cmp::Ordering::Equal && opts.mode != SnippetSort::Relevance {
            match (&a.fuzzy_score, &b.fuzzy_score) {
                (Some(sa), Some(sb)) => match sb.cmp(sa) {
                    std::cmp::Ordering::Equal => {}
                    other => return other,
                },
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => {}
            }
        }

        // 4. Normalized description (skipped for Relevance mode — the spec
        //    defines Relevance as: fuzzy score → index directly)
        if primary == std::cmp::Ordering::Equal && opts.mode != SnippetSort::Relevance {
            let desc_tie = a.desc_lower.cmp(&b.desc_lower);
            if desc_tie != std::cmp::Ordering::Equal {
                return desc_tie;
            }
        }

        // 5. Original index (stable)
        if primary == std::cmp::Ordering::Equal {
            a.index.cmp(&b.index)
        } else {
            primary
        }
    });

    sorted.into_iter().map(|r| r.index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Snippet;
    use std::collections::HashMap;

    fn snippet(id: &str, desc: &str, cmd: &str, favorite: bool) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            favorite,
            ..Default::default()
        }
    }

    fn snippet_with_timestamps(
        id: &str,
        desc: &str,
        cmd: &str,
        favorite: bool,
        updated_at: i64,
        created_at: i64,
    ) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            favorite,
            updated_at,
            created_at,
            ..Default::default()
        }
    }

    fn usage(count: u64, last_used_at: Option<i64>) -> crate::usage::UsageData {
        crate::usage::UsageData {
            use_count: count,
            last_used_at,
        }
    }

    fn default_indices(n: usize) -> Vec<usize> {
        (0..n).collect()
    }

    // ── Relevance sort ──────────────────────────────────────────────

    #[test]
    fn relevance_sorts_by_fuzzy_score_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 10);
        scores.insert(1, 30);
        scores.insert(2, 20);

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, Some(&scores), None, &opts);
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn relevance_none_scores_sort_after_some() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let mut scores = HashMap::new();
        scores.insert(1, 50);
        // indices 0 and 2 have no score

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, Some(&scores), None, &opts);
        // index 1 (score 50) first, then 0 and 2 tied at None → index order
        assert_eq!(result, vec![1, 0, 2]);
    }

    #[test]
    fn relevance_tie_breaks_by_original_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 10);
        scores.insert(1, 10);
        scores.insert(2, 10);

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, Some(&scores), None, &opts);
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── Recent sort ─────────────────────────────────────────────────

    #[test]
    fn recent_sorts_by_updated_at_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 100, 50),
            snippet_with_timestamps("b", "B", "cmd b", false, 300, 50),
            snippet_with_timestamps("c", "C", "cmd c", false, 200, 50),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Recent,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn recent_tie_breaks_by_created_at_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 100, 200),
            snippet_with_timestamps("b", "B", "cmd b", false, 100, 300),
            snippet_with_timestamps("c", "C", "cmd c", false, 100, 100),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Recent,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 0, 2]);
    }

    #[test]
    fn recent_tie_breaks_by_index_when_all_equal() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 100, 100),
            snippet_with_timestamps("b", "B", "cmd b", false, 100, 100),
            snippet_with_timestamps("c", "C", "cmd c", false, 100, 100),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Recent,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── LastUsed sort ───────────────────────────────────────────────

    #[test]
    fn last_used_sorts_by_last_used_at_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(1, Some(300)),
            usage(1, Some(200)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn last_used_none_after_some() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![usage(1, None), usage(1, Some(300)), usage(1, None)];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![1, 0, 2]);
    }

    #[test]
    fn last_used_tie_breaks_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
        ];
        let usage_data = vec![usage(1, Some(100)), usage(1, Some(100))];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(2),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![0, 1]);
    }

    // ── MostUsed sort ───────────────────────────────────────────────

    #[test]
    fn most_used_sorts_by_use_count_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![usage(5, None), usage(20, None), usage(10, None)];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn most_used_tie_breaks_by_last_used_at_descending() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(10, Some(100)),
            usage(10, Some(300)),
            usage(10, Some(200)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn most_used_tie_breaks_by_index_when_all_equal() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(10, Some(100)),
            usage(10, Some(100)),
            usage(10, Some(100)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── Description sort ────────────────────────────────────────────

    #[test]
    fn description_sorts_case_insensitive() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Banana", "cmd a", false),
            snippet("b", "apple", "cmd b", false),
            snippet("c", "Cherry", "cmd c", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 0, 2]);
    }

    #[test]
    fn description_tie_breaks_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "same", "cmd a", false),
            snippet("b", "same", "cmd b", false),
            snippet("c", "same", "cmd c", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── Command sort ────────────────────────────────────────────────

    #[test]
    fn command_sorts_case_insensitive() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "Zebra", false),
            snippet("b", "B", "alpha", false),
            snippet("c", "C", "MIXED", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Command,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn command_tie_breaks_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "same", false),
            snippet("b", "B", "same", false),
            snippet("c", "C", "same", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Command,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── Favorites-first grouping ────────────────────────────────────

    #[test]
    fn favorites_first_groups_favorites_before_non_favorites() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Zebra", "cmd a", false),
            snippet("b", "Alpha", "cmd b", true),
            snippet("c", "Middle", "cmd c", false),
            snippet("d", "Beta", "cmd d", true),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(4), &snippets, None, None, &opts);
        // Favorites: Alpha (b), Beta (d) — description sort within favorites
        // Non-favorites: Middle (c), Zebra (a) — description sort within non-favorites
        assert_eq!(result, vec![1, 3, 2, 0]);
    }

    #[test]
    fn favorites_first_with_relevance() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", true),
            snippet("c", "C", "cmd c", false),
            snippet("d", "D", "cmd d", true),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 30); // non-fav, high score
        scores.insert(1, 10); // fav, low score
        scores.insert(2, 20); // non-fav, mid score
        scores.insert(3, 5); // fav, lowest score

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(4), &snippets, Some(&scores), None, &opts);
        // Fav group: b (10), d (5) — relevance desc within favorites
        // Non-fav group: a (30), c (20) — relevance desc within non-favorites
        assert_eq!(result, vec![1, 3, 0, 2]);
    }

    #[test]
    fn favorites_first_all_favorites() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Zebra", "cmd a", true),
            snippet("b", "Alpha", "cmd b", true),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(2), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 0]);
    }

    #[test]
    fn favorites_first_none_favorites() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Zebra", "cmd a", false),
            snippet("b", "Alpha", "cmd b", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(2), &snippets, None, None, &opts);
        assert_eq!(result, vec![1, 0]);
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn empty_input_returns_empty() {
        let snippets: Vec<Snippet> = vec![];
        let opts = SortOptions::default();
        let result = rank_snippets(&[], &snippets, None, None, &opts);
        assert!(result.is_empty());
    }

    #[test]
    fn single_element_returns_that_index() {
        let snippets: Vec<Snippet> = vec![snippet("a", "A", "cmd a", false)];
        let opts = SortOptions::default();
        let result = rank_snippets(&[0], &snippets, None, None, &opts);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn all_equal_elements_stable_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "same", "same", false),
            snippet("b", "same", "same", false),
            snippet("c", "same", "same", false),
            snippet("d", "same", "same", false),
        ];

        let opts = SortOptions::default();
        let result = rank_snippets(&default_indices(4), &snippets, None, None, &opts);
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn custom_index_order_preserved_as_input() {
        // rank_snippets sorts; it doesn't just return the input.  But
        // non-contiguous input indices are handled correctly.
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Zebra", "cmd a", false),
            snippet("b", "Alpha", "cmd b", false),
            snippet("c", "Middle", "cmd c", false),
        ];
        let input = vec![2, 0]; // skip index 1

        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&input, &snippets, None, None, &opts);
        // Among {2, 0}: Alpha isn't present; Middle (2) < Zebra (0)
        assert_eq!(result, vec![2, 0]);
    }

    #[test]
    fn favorites_first_single_favorite() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Alpha", "cmd a", true),
            snippet("b", "Beta", "cmd b", false),
            snippet("c", "Gamma", "cmd c", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(3), &snippets, None, None, &opts);
        // Favorite: a (Alpha)
        // Non-favorites: b (Beta), c (Gamma)
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn description_tie_breaks_after_recent_primary() {
        // Two snippets with same updated_at and created_at — description
        // tie-break should apply.
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "Zebra", "cmd a", false, 100, 100),
            snippet_with_timestamps("b", "Alpha", "cmd b", false, 100, 100),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Recent,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(2), &snippets, None, None, &opts);
        // Both have same timestamps → description tie-break: Alpha < Zebra
        assert_eq!(result, vec![1, 0]);
    }

    #[test]
    fn relevance_primary_does_not_use_description_tie_break() {
        // When Relevance is primary, equal fuzzy scores fall through to
        // index. Description is not consulted.
        let snippets: Vec<Snippet> = vec![
            snippet("a", "Zebra", "cmd a", false),
            snippet("b", "Alpha", "cmd b", false),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 10);
        scores.insert(1, 10);

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(2), &snippets, Some(&scores), None, &opts);
        // Equal scores → index order (not description)
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn most_used_none_last_used_treats_as_equal() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
        ];
        let usage_data = vec![usage(5, None), usage(5, None)];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(2),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Equal counts, both None last_used → index order
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn sort_options_default_is_relevance() {
        let opts = SortOptions::default();
        assert_eq!(opts.mode, SnippetSort::Relevance);
        assert!(!opts.favorites_first);
    }

    #[test]
    fn description_sort_is_case_insensitive_with_mixed_case() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "APPLE", "cmd", false),
            snippet("b", "banana", "cmd", false),
            snippet("c", "Cherry", "cmd", false),
            snippet("d", "date", "cmd", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(4), &snippets, None, None, &opts);
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn last_used_all_none_tie_breaks_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![usage(1, None), usage(1, None), usage(1, None)];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn favorites_first_with_equal_descriptions_in_groups() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "same", "cmd a", true),
            snippet("b", "same", "cmd b", false),
            snippet("c", "same", "cmd c", true),
            snippet("d", "same", "cmd d", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            favorites_first: true,
        };
        let result = rank_snippets(&default_indices(4), &snippets, None, None, &opts);
        // Fav: a, c (same desc → index order)
        // Non-fav: b, d (same desc → index order)
        assert_eq!(result, vec![0, 2, 1, 3]);
    }

    // ── Favorites-first + usage sort combinations ──────────────────

    #[test]
    fn favorites_first_with_last_used() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", true),
            snippet("c", "C", "cmd c", false),
            snippet("d", "D", "cmd d", true),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(1, Some(300)),
            usage(1, Some(200)),
            usage(1, Some(50)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            favorites_first: true,
        };
        let result = rank_snippets(
            &default_indices(4),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Fav: b (300), d (50) — last_used desc within favorites
        // Non-fav: c (200), a (100) — last_used desc within non-favorites
        assert_eq!(result, vec![1, 3, 2, 0]);
    }

    #[test]
    fn favorites_first_with_most_used() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", true),
            snippet("c", "C", "cmd c", false),
            snippet("d", "D", "cmd d", true),
        ];
        let usage_data = vec![
            usage(5, Some(100)),
            usage(10, Some(300)),
            usage(3, Some(200)),
            usage(15, Some(50)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            favorites_first: true,
        };
        let result = rank_snippets(
            &default_indices(4),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Fav: d (15), b (10) — use_count desc within favorites
        // Non-fav: a (5), c (3) — use_count desc within non-favorites
        assert_eq!(result, vec![3, 1, 0, 2]);
    }

    // ── Never-used snippets sort after used snippets ───────────────

    #[test]
    fn last_used_never_used_after_used() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(1, None),      // never used
            usage(1, Some(200)), // used
            usage(1, None),      // never used
        ];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // b (used) first, then a, c (never used, index order)
        assert_eq!(result, vec![1, 0, 2]);
    }

    #[test]
    fn most_used_never_used_after_used() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(0, None),      // never used (count=0)
            usage(5, Some(200)), // used 5 times
            usage(0, None),      // never used
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // b (5 uses) first, then a, c (0 uses, index order)
        assert_eq!(result, vec![1, 0, 2]);
    }

    // ── Equal counts tie-break by last_used_at ─────────────────────

    #[test]
    fn most_used_equal_counts_tie_by_last_used_at() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        let usage_data = vec![
            usage(5, Some(100)),
            usage(5, Some(300)),
            usage(5, Some(200)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // All equal count=5, tie by last_used_at desc: b (300), c (200), a (100)
        assert_eq!(result, vec![1, 2, 0]);
    }

    // ── Divergent metadata fixture ─────────────────────────────────

    /// Fixture from the plan: updated_at, use_count, and last_used_at all disagree.
    /// Snippet A: updated_at=300, use_count=1, last_used_at=100
    /// Snippet B: updated_at=100, use_count=9, last_used_at=200
    /// Snippet C: updated_at=200, use_count=2, last_used_at=900
    #[test]
    fn divergent_metadata_recent_order() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 300, 100),
            snippet_with_timestamps("b", "B", "cmd b", false, 100, 50),
            snippet_with_timestamps("c", "C", "cmd c", false, 200, 50),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(9, Some(200)),
            usage(2, Some(900)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Recent,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Recent: by updated_at desc → A (300), C (200), B (100)
        assert_eq!(result, vec![0, 2, 1]);
    }

    #[test]
    fn divergent_metadata_most_used_order() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 300, 100),
            snippet_with_timestamps("b", "B", "cmd b", false, 100, 50),
            snippet_with_timestamps("c", "C", "cmd c", false, 200, 50),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(9, Some(200)),
            usage(2, Some(900)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::MostUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Most-used: by use_count desc → B (9), C (2), A (1)
        assert_eq!(result, vec![1, 2, 0]);
    }

    #[test]
    fn divergent_metadata_last_used_order() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "A", "cmd a", false, 300, 100),
            snippet_with_timestamps("b", "B", "cmd b", false, 100, 50),
            snippet_with_timestamps("c", "C", "cmd c", false, 200, 50),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(9, Some(200)),
            usage(2, Some(900)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // Last-used: by last_used_at desc → C (900), B (200), A (100)
        assert_eq!(result, vec![2, 1, 0]);
    }

    // ── Default relevance tie: no usage influence ───────────────────

    #[test]
    fn relevance_equal_scores_tie_by_index_not_usage() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 10);
        scores.insert(1, 10);
        let usage_data = vec![
            usage(100, Some(900)), // heavily used
            usage(0, None),        // never used
        ];

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(2),
            &snippets,
            Some(&scores),
            Some(&usage_data),
            &opts,
        );
        // Equal scores → index order, NOT usage order
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn relevance_none_scores_sorted_by_index_not_usage() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        // No fuzzy scores at all (no filter active)
        let usage_data = vec![usage(100, Some(900)), usage(0, None), usage(50, Some(500))];

        let opts = SortOptions {
            mode: SnippetSort::Relevance,
            ..Default::default()
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        // All None scores → index order, usage has no effect
        assert_eq!(result, vec![0, 1, 2]);
    }

    // ── Deleted snippets absent from candidates ────────────────────

    #[test]
    fn deleted_snippets_not_in_candidate_indices() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "A", "cmd a", false),
            snippet("b", "B", "cmd b", false),
            snippet("c", "C", "cmd c", false),
        ];
        // Only pass indices 0 and 2 (skipping deleted index 1)
        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&[0, 2], &snippets, None, None, &opts);
        // Should only contain indices 0 and 2
        assert!(result.contains(&0));
        assert!(result.contains(&2));
        assert!(!result.contains(&1));
        assert_eq!(result.len(), 2);
    }

    // ── Duplicate descriptions remain distinct by ID/index ─────────

    #[test]
    fn duplicate_descriptions_distinct_by_index() {
        let snippets: Vec<Snippet> = vec![
            snippet("a", "same", "cmd a", false),
            snippet("b", "same", "cmd b", false),
        ];

        let opts = SortOptions {
            mode: SnippetSort::Description,
            ..Default::default()
        };
        let result = rank_snippets(&default_indices(2), &snippets, None, None, &opts);
        // Both have same description → index order
        assert_eq!(result, vec![0, 1]);
    }

    // ── CLI/TUI mode mapping exhaustiveness ────────────────────────

    #[test]
    fn explicit_sort_breaks_fuzzy_score_ties() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "Alpha", "echo a", false, 300, 100),
            snippet_with_timestamps("b", "Alpha", "echo b", false, 100, 300),
            snippet_with_timestamps("c", "Alpha", "echo c", false, 200, 200),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(3, Some(300)),
            usage(2, Some(200)),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 100);
        scores.insert(1, 100);
        scores.insert(2, 100);

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            favorites_first: false,
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            Some(&scores),
            Some(&usage_data),
            &opts,
        );
        assert_eq!(
            result,
            vec![1, 2, 0],
            "equal fuzzy scores should defer to LastUsed (b has last_used_at 300, c has 200, a has 100)"
        );
    }

    #[test]
    fn explicit_sort_with_none_fuzzy_scores_uses_explicit_sort() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "Alpha", "echo a", false, 300, 100),
            snippet_with_timestamps("b", "Alpha", "echo b", false, 100, 300),
            snippet_with_timestamps("c", "Alpha", "echo c", false, 200, 200),
        ];
        let usage_data = vec![
            usage(1, Some(100)),
            usage(3, Some(300)),
            usage(2, Some(200)),
        ];

        let opts = SortOptions {
            mode: SnippetSort::LastUsed,
            favorites_first: false,
        };
        let result = rank_snippets(
            &default_indices(3),
            &snippets,
            None,
            Some(&usage_data),
            &opts,
        );
        assert_eq!(
            result,
            vec![1, 2, 0],
            "None fuzzy scores + LastUsed should sort by last_used_at descending"
        );
    }

    #[test]
    fn all_sort_variants_produce_deterministic_output() {
        let snippets: Vec<Snippet> = vec![
            snippet_with_timestamps("a", "Zebra", "echo z", false, 300, 100),
            snippet_with_timestamps("b", "Alpha", "echo a", false, 100, 200),
            snippet_with_timestamps("c", "Middle", "echo m", false, 200, 150),
        ];
        let usage_data = vec![
            usage(5, Some(300)),
            usage(1, Some(100)),
            usage(3, Some(200)),
        ];
        let mut scores = HashMap::new();
        scores.insert(0, 10);
        scores.insert(1, 30);
        scores.insert(2, 20);

        for mode in [
            SnippetSort::Relevance,
            SnippetSort::Recent,
            SnippetSort::LastUsed,
            SnippetSort::MostUsed,
            SnippetSort::Description,
            SnippetSort::Command,
        ] {
            let opts = SortOptions {
                mode,
                favorites_first: false,
            };
            let r1 = rank_snippets(
                &default_indices(3),
                &snippets,
                Some(&scores),
                Some(&usage_data),
                &opts,
            );
            let r2 = rank_snippets(
                &default_indices(3),
                &snippets,
                Some(&scores),
                Some(&usage_data),
                &opts,
            );
            assert_eq!(r1, r2, "sort mode {mode:?} must be deterministic");
        }
    }
}
