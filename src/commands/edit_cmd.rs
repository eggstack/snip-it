use crate::commands::get_library_path;
use crate::error::{SnipError, SnipResult};
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;

/// Opens the snippets library file in the user's `$EDITOR`.
pub fn run(library: Option<String>, _config: Option<PathBuf>) -> SnipResult<()> {
    let path = if let Some(ref lib_name) = library {
        match get_library_path(library.clone())? {
            Some(p) => p,
            None => {
                eprintln!(
                    "Library '{lib_name}' not found. Use 'snp library list' to see available libraries."
                );
                return Err(crate::error::SnipError::runtime_error(
                    "Library not found",
                    Some(&format!("Library '{lib_name}' does not exist")),
                ));
            }
        }
    } else {
        get_library_path(None)?
            .unwrap_or_else(crate::library::LibraryManager::get_default_snippets_path)
    };
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        File::create(&path)?;
    }
    let editor = crate::commands::new_cmd::resolve_editor_spec()?;
    let editor_label = editor.program_label();

    // Snapshot the exact pre-editor bytes so we can compare after the
    // editor exits. Mutation notification must reflect whether bytes
    // actually changed, independent of the editor's exit status.
    let before = fs::read(&path)?;

    // The editor rewrites snippets.toml in place, so this is a local
    // mutating operation: refuse when interrupted transactions await
    // recovery (same invariant as every other writer).
    let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
    let transaction_dir = crate::local_data::derive_local_data_state_dir();
    crate::transaction::gate_mutation_on_interrupted_transactions(
        &sync_state_dir,
        &transaction_dir,
    )?;

    let status = Command::new(&editor.program)
        .args(&editor.args)
        .arg(&path)
        .status()
        .map_err(|e| {
            SnipError::command_error(
                &editor_label,
                editor
                    .args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .chain(std::iter::once(path.display().to_string()))
                    .collect(),
                e,
            )
        })?;

    let after = fs::read(&path)?;

    // Serialize the observe-and-notify window against other local writers
    // (backup capture, concurrent saves). The interactive editor session
    // itself cannot hold the lock; the short post-editor critical section
    // is where consistency matters.
    let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;

    let changed = before != after;

    if !status.success() {
        if changed {
            crate::auto_sync::report_notification_result(crate::auto_sync::notify_mutation(
                crate::auto_sync::MutationKind::SnippetUpdate,
                crate::auto_sync::MutationOrigin::User,
            ));
        }
        return Err(SnipError::runtime_error(
            "Editor failed",
            Some(&format!(
                "EDITOR '{}' exited with non-zero status {:?}.{}",
                editor_label,
                status.code(),
                if changed {
                    " The library was modified; saved changes were notified for sync."
                } else {
                    " The library was not modified."
                }
            )),
        ));
    }

    // Auto-sync trigger: notify only when the library actually changed.
    // Unchanged editor sessions must not create pending sync intent.
    if changed {
        crate::auto_sync::report_notification_result(crate::auto_sync::notify_mutation(
            crate::auto_sync::MutationKind::SnippetUpdate,
            crate::auto_sync::MutationOrigin::User,
        ));
    }

    Ok(())
}

/// Edits the output/notes field of a single snippet matched by filter.
///
/// When `new_output` is `Some(value)`, the output field is set to that value.
/// When `new_output` is `None` (shouldn't happen, but defensive), the field is cleared.
pub fn run_edit_output(
    library: Option<String>,
    filter: String,
    new_output: Option<String>,
) -> SnipResult<()> {
    use crate::commands::get_library_path;
    use crate::library::{load_library, save_library};

    let lib_path = match get_library_path(library.clone())? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Err(crate::error::SnipError::runtime_error(
                "Library not found",
                Some("No library available"),
            ));
        }
    };

    let mut snippets = load_library(&lib_path)?;

    // Find the snippets matching the filter. Substring matching can easily
    // hit several snippets, and this path mutates a local-only field —
    // guessing would silently edit the wrong snippet's output.
    let filter_lower = filter.to_lowercase();
    let matching: Vec<usize> = snippets
        .snippets
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.deleted)
        .filter(|(_, s)| {
            s.description.to_lowercase().contains(&filter_lower)
                || s.command.to_lowercase().contains(&filter_lower)
        })
        .map(|(i, _)| i)
        .collect();

    let idx = match matching.as_slice() {
        [only] => *only,
        [] => {
            return Err(crate::error::SnipError::runtime_error(
                "No matching snippet",
                Some(&format!("No snippet matching '{filter}' found in library")),
            ));
        }
        many => {
            let candidates = many
                .iter()
                .map(|&i| {
                    let s = &snippets.snippets[i];
                    format!("  - {} (id: {})", s.description, s.id)
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(crate::error::SnipError::runtime_error(
                "Ambiguous filter match",
                Some(&format!(
                    "Filter '{filter}' matches {} snippets:\n{candidates}\n\
                     Refine the filter or use an exact selector \
                     (--id / --description-exact / --command-exact).",
                    many.len()
                )),
            ));
        }
    };

    let snippet = &mut snippets.snippets[idx];
    snippet.output = new_output.unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    snippet.updated_at = snippet.updated_at.max(now).saturating_add(1);
    let desc = snippet.description.clone();
    let is_empty = snippet.output.is_empty();

    save_library(&lib_path, &snippets)?;

    if is_empty {
        eprintln!("Cleared output for snippet: {desc}");
    } else {
        eprintln!("Updated output for snippet: {desc}");
    }

    Ok(())
}

/// Edits the output/notes field of a snippet identified by stable ID.
///
/// Unlike `run_edit_output` which searches by description/command text,
/// this function uses the snippet's unique ID for precise targeting.
/// This is the correct mutation path for exact selectors (`--id`,
/// `--description-exact`, `--command-exact`) to avoid identity loss
/// when multiple snippets share similar descriptions.
pub fn run_edit_output_by_id(
    library: Option<String>,
    snippet_id: &str,
    new_output: Option<String>,
) -> SnipResult<()> {
    use crate::commands::get_library_path;
    use crate::library::{load_library, save_library};

    let lib_path = match get_library_path(library.clone())? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Err(crate::error::SnipError::runtime_error(
                "Library not found",
                Some("No library available"),
            ));
        }
    };

    let mut snippets = load_library(&lib_path)?;

    let matching_idx = snippets
        .snippets
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.deleted)
        .find(|(_, s)| s.id == snippet_id)
        .map(|(i, _)| i);

    let idx = match matching_idx {
        Some(i) => i,
        None => {
            return Err(crate::error::SnipError::runtime_error(
                "Snippet not found",
                Some(&format!(
                    "No snippet with ID '{snippet_id}' found in library"
                )),
            ));
        }
    };

    let snippet = &mut snippets.snippets[idx];
    snippet.output = new_output.unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    snippet.updated_at = snippet.updated_at.max(now).saturating_add(1);
    let desc = snippet.description.clone();
    let is_empty = snippet.output.is_empty();

    save_library(&lib_path, &snippets)?;

    if is_empty {
        eprintln!("Cleared output for snippet: {desc}");
    } else {
        eprintln!("Updated output for snippet: {desc}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {}
