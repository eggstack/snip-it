use crate::error::SnipResult;
use crate::library::LibraryManager;
use std::io::IsTerminal;

pub fn run_list() -> SnipResult<()> {
    let mgr = LibraryManager::new()?;
    let libraries = mgr.list_libraries();

    if libraries.is_empty() {
        println!("No libraries found.");
        return Ok(());
    }

    println!("Libraries:");
    for lib in libraries {
        let primary = if lib.is_primary { " (primary)" } else { "" };
        println!("  {}{}", lib.filename, primary);
    }
    Ok(())
}

pub fn run_create(name: String) -> SnipResult<()> {
    let mut mgr = LibraryManager::new()?;
    let path = mgr.create_library(&name)?;
    println!("Created library '{}' at {}", name, path.display());
    Ok(())
}

pub fn run_delete(name: String, force: bool) -> SnipResult<()> {
    let mut mgr = LibraryManager::new()?;

    if !force {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "Refusing to delete library '{}' in non-interactive mode. Use --force to override.",
                name
            );
            return Err(crate::error::SnipError::runtime_error(
                "Non-interactive delete",
                Some("Use --force to delete libraries in non-interactive mode"),
            ));
        }
        eprint!(
            "Are you sure you want to delete library '{}'? [y/N]: ",
            name
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        if input.trim().to_lowercase() != "y" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    mgr.delete_library(&name)?;
    println!("Deleted library '{}'", name);
    Ok(())
}

pub fn run_set_primary(name: String) -> SnipResult<()> {
    let mut mgr = LibraryManager::new()?;
    mgr.set_primary(&name)?;
    println!("Set '{}' as primary library", name);
    Ok(())
}

pub fn run_show(name: Option<String>) -> SnipResult<()> {
    let mgr = LibraryManager::new()?;

    if let Some(name) = name {
        if let Some(lib) = mgr.get_library_by_filename(&name) {
            println!("Library: {}", lib.filename);
            println!("  ID: {}", lib.library_id.if_empty("{not linked}"));
            println!("  Primary: {}", lib.is_primary);
            if let Some(ts) = lib.last_sync {
                println!(
                    "  Last sync: {}",
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
        } else {
            eprintln!("Library '{}' not found", name);
        }
    } else {
        println!("Libraries:");
        for lib in mgr.list_libraries() {
            let primary = if lib.is_primary { " (primary)" } else { "" };
            let linked = if lib.library_id.is_empty() {
                ""
            } else {
                " [linked]"
            };
            println!("  {}{}{}", lib.filename, primary, linked);
        }
    }
    Ok(())
}

trait StringExt {
    fn if_empty(&self, fallback: &str) -> String;
}

impl StringExt for String {
    fn if_empty(&self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_if_empty_returns_fallback_when_empty() {
        let s = String::new();
        assert_eq!(s.if_empty("default"), "default");
    }

    #[test]
    fn test_if_empty_returns_self_when_not_empty() {
        let s = "hello".to_string();
        assert_eq!(s.if_empty("default"), "hello");
    }

    #[test]
    fn test_if_empty_with_empty_fallback() {
        let s = String::new();
        assert_eq!(s.if_empty(""), "");
    }
}
