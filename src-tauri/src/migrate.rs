//! One-time move from the app's former name (Coucou) to OpenJarvis.
//!
//! Runs first thing at startup, before anything reads or writes a data file:
//! - the config and local data folders are moved to their new names (settings,
//!   Jarvis memories, AI accounts, the Jarvis browser profile, the log…).
//! Keychain secrets migrate lazily in secrets.rs; Claude Code / agent hooks are
//! re-pointed at the renamed relay by `hooks()` once the relay is installed.
//! Nothing is ever deleted: a folder that cannot be moved is copied instead,
//! and an old folder is only removed once it is empty.

use std::fs;
use std::path::{Path, PathBuf};

/// The former name, spelled so the rename script that produced this code
/// cannot turn it into the new one.
const OLD_TITLE: &str = concat!("Cou", "cou");
const OLD_LOWER: &str = concat!("cou", "cou");
/// The relay's former file name, as it appears in hook commands.
pub const OLD_RELAY: &str = concat!("cou", "cou", "-hook");

/// `…\OpenJarvis` → `…\Coucou`, `…/openjarvis` → `…/coucou`.
fn legacy_sibling(new_dir: &Path) -> Option<PathBuf> {
    let name = new_dir.file_name()?.to_str()?;
    let old = match name {
        "OpenJarvis" => OLD_TITLE,
        "openjarvis" => OLD_LOWER,
        _ => return None,
    };
    Some(new_dir.with_file_name(old))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if dst.exists() {
            continue; // never overwrite what the new folder already has
        }
        if entry.file_type()?.is_dir() {
            let _ = copy_tree(&src, &dst);
        } else {
            let _ = fs::copy(&src, &dst);
        }
    }
    Ok(())
}

/// Moves `old` to `new`. When `new` already exists (something wrote to it
/// first), the entries it lacks are moved in one by one. Returns what happened.
fn move_dir(old: &Path, new: &Path) -> Option<String> {
    if !old.is_dir() {
        return None;
    }
    if !new.exists() {
        if let Some(parent) = new.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::rename(old, new).is_ok() {
            return Some(format!("moved {} -> {}", old.display(), new.display()));
        }
        // Locked (another process has a file open): copy instead, keep the old one.
        return Some(match copy_tree(old, new) {
            Ok(()) => format!("copied {} -> {} (could not move)", old.display(), new.display()),
            Err(e) => format!("could not migrate {}: {e}", old.display()),
        });
    }
    let mut moved = 0;
    if let Ok(entries) = fs::read_dir(old) {
        for entry in entries.flatten() {
            let dst = new.join(entry.file_name());
            if dst.exists() {
                continue;
            }
            if fs::rename(entry.path(), &dst).is_ok() {
                moved += 1;
            } else if entry.path().is_dir() {
                let _ = copy_tree(&entry.path(), &dst);
                moved += 1;
            } else if fs::copy(entry.path(), &dst).is_ok() {
                moved += 1;
            }
        }
    }
    // Only an empty folder is removed.
    let _ = fs::remove_dir(old);
    (moved > 0).then(|| format!("merged {moved} item(s) from {} into {}", old.display(), new.display()))
}

/// Folders and the WebView2 profile. Call before anything touches a data dir.
pub fn data() -> Vec<String> {
    let mut done = Vec::new();
    for new in [crate::platform::config_dir(), crate::platform::local_dir()] {
        if let Some(old) = legacy_sibling(&new) {
            if old != new {
                done.extend(move_dir(&old, &new));
            }
        }
    }
    // The WebView2 profile follows the Tauri identifier (com.jarvis.assistant),
    // which the rename did not change: nothing to move there.
    done
}

/// Hook commands still pointing at the old relay are re-pointed at the new
/// one (reinstalling replaces OpenJarvis's own entries and nothing else).
pub fn hooks() {
    // Never point hooks at a relay that is not on disk.
    if !crate::settings::hook_exe_path().exists() {
        crate::log::line("hook migration skipped: the renamed relay is not installed yet".to_string());
        return;
    }
    let settings = crate::hooks::settings_path();
    let text = fs::read_to_string(&settings).unwrap_or_default();
    if text.contains(OLD_RELAY) {
        let mut parts = Vec::new();
        if text.contains(&format!("{OLD_RELAY}\" ")) || text.contains("\"hooks\"") {
            match crate::hooks::preview(true).and_then(|p| crate::hooks::write(true, &p.fingerprint)) {
                Ok(_) => parts.push("Claude Code hooks"),
                Err(e) => crate::log::line(format!("hook migration failed: {e}")),
            }
        }
        // The status line relay (plan usage), when it is ours.
        let text = fs::read_to_string(&settings).unwrap_or_default();
        if text.contains(OLD_RELAY) {
            match crate::hooks::status_line_preview(true)
                .and_then(|p| crate::hooks::status_line_write(true, &p.fingerprint))
            {
                Ok(_) => parts.push("status line"),
                Err(e) => crate::log::line(format!("status line migration failed: {e}")),
            }
        }
        if !parts.is_empty() {
            crate::log::line(format!("re-pointed {} at the renamed relay", parts.join(" + ")));
        }
    }
    // Other agents (Cursor, Gemini CLI, Codex…): reinstall the ones that are
    // installed, once — their configs may hold the old relay path.
    let marker = crate::platform::config_dir().join(".renamed-agents-done");
    if marker.exists() {
        return;
    }
    for agent in crate::agents::list() {
        if !agent.installed {
            continue;
        }
        match crate::agents::preview(&agent.id, true).and_then(|p| crate::agents::apply(&agent.id, true, &p.fingerprint)) {
            Ok(_) => crate::log::line(format!("re-pointed {} hooks at the renamed relay", agent.id)),
            Err(e) => crate::log::line(format!("agent {} hook migration failed: {e}", agent.id)),
        }
    }
    let _ = fs::write(marker, b"1");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oj-migrate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_old_folder_name_is_found_next_to_the_new_one() {
        assert_eq!(legacy_sibling(Path::new(r"C:\Users\a\AppData\Roaming\OpenJarvis")).unwrap(), PathBuf::from(r"C:\Users\a\AppData\Roaming").join(OLD_TITLE));
        assert_eq!(legacy_sibling(Path::new("/home/a/.config/openjarvis")).unwrap(), PathBuf::from("/home/a/.config").join(OLD_LOWER));
        assert!(legacy_sibling(Path::new("/tmp/other")).is_none());
    }

    #[test]
    fn a_folder_is_moved_whole_when_the_new_one_does_not_exist() {
        let root = scratch("whole");
        let old = root.join(OLD_TITLE);
        fs::create_dir_all(old.join("bin")).unwrap();
        fs::write(old.join("settings.json"), "{}").unwrap();
        let new = root.join("OpenJarvis");
        assert!(move_dir(&old, &new).unwrap().starts_with("moved"));
        assert!(new.join("settings.json").exists() && new.join("bin").is_dir());
        assert!(!old.exists());
    }

    #[test]
    fn an_existing_new_folder_only_receives_what_it_lacks() {
        let root = scratch("merge");
        let old = root.join(OLD_TITLE);
        let new = root.join("OpenJarvis");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("settings.json"), "old").unwrap();
        fs::write(old.join("jarvis_data.json"), "data").unwrap();
        fs::write(new.join("settings.json"), "new").unwrap();
        move_dir(&old, &new);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), "new");
        assert_eq!(fs::read_to_string(new.join("jarvis_data.json")).unwrap(), "data");
    }

    #[test]
    fn nothing_happens_without_an_old_folder() {
        let root = scratch("none");
        assert!(move_dir(&root.join(OLD_TITLE), &root.join("OpenJarvis")).is_none());
    }
}
