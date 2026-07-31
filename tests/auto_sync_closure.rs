//! Focused contracts for the single-helper auto-sync architecture.

mod support;

use snip_it::auto_sync::pending::{self, ConditionalClearResult, PendingSnapshot};
use snip_it::auto_sync::policy::MutationKind;
use tempfile::TempDir;

#[test]
fn exact_generation_clear_preserves_newer_work() {
    let dir = TempDir::new().unwrap();
    let first = pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap();
    pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetUpdate,
        },
    )
    .unwrap();
    assert!(matches!(
        pending::clear_if_generation_matches(dir.path(), first.generation).unwrap(),
        ConditionalClearResult::GenerationChanged { .. }
    ));
    assert_eq!(
        pending::read_state_from_dir(dir.path()).unwrap().generation,
        first.generation + 1
    );
}
