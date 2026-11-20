//! Notas: search, read and write the user's Obsidian vault ("anota isso no
//! projeto jarvis"). The vault is the folder in Ajustes (or its parent when
//! that parent is the actual vault, holding `.obsidian`).

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::file_search::fold;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteHit {
    /// Relative to the vault, with forward slashes.
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub modified: u64,
}

pub fn vault() -> PathBuf {
    let dir = crate::jarvis_config::obsidian_vault_dir();
    let mut cur = Some(dir.as_path());
    while let Some(d) = cur {
        if d.join(".obsidian").is_dir() {
            return d.to_path_buf();
        }
        cur = d.parent();
    }
    dir
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 || out.len() > 20_000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else if name.to_lowercase().ends_with(".md") {
            out.push(p);
        }
    }
}

fn modified(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// A line around the first match, for the result list.
pub fn snippet(text: &str, words: &[String]) -> String {
    let line = text
        .lines()
        .find(|l| {
            let f = fold(l);
            words.iter().any(|w| f.contains(w.as_str()))
        })
        .or_else(|| text.lines().find(|l| !l.trim().is_empty() && !l.starts_with("---")))
        .unwrap_or("")
        .trim();
    let mut s: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        s.push('…');
    }
    s
}

/// Notes whose title or text has every word; title matches first, then newest.
/// An empty query lists the most recently edited notes.
pub fn search(query: &str, limit: usize) -> Result<Vec<NoteHit>, String> {
    let root = vault();
    if !root.is_dir() {
        return Err(format!("Vault do Obsidian não encontrado: {}. Ajuste em Ajustes → IA & Voz.", root.display()));
    }
    let mut files = Vec::new();
    walk(&root, &mut files, 0);
    let words: Vec<String> = fold(query).split_whitespace().filter(|w| w.len() > 1).map(String::from).collect();
    let mut hits: Vec<(i32, NoteHit)> = Vec::new();
    for f in files {
        let title = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ft = fold(&title);
        let mut score = 0;
        let mut text = String::new();
        if !words.is_empty() {
            let in_title = words.iter().filter(|w| ft.contains(w.as_str())).count();
            text = std::fs::read_to_string(&f).unwrap_or_default();
            let body = fold(&text);
            if !words.iter().all(|w| ft.contains(w.as_str()) || body.contains(w.as_str())) {
                continue;
            }
            score = in_title as i32 * 10 + 1;
        }
        let m = modified(&f);
        if text.is_empty() && hits.len() < limit * 4 {
            text = std::fs::read_to_string(&f).unwrap_or_default();
        }
        hits.push((score, NoteHit { path: rel(&root, &f), title, snippet: snippet(&text, &words), modified: m }));
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    Ok(hits.into_iter().take(limit).map(|(_, h)| h).collect())
}

/// Resolves a vault-relative path, refusing anything outside the vault.
fn inside(root: &Path, relp: &str) -> Result<PathBuf, String> {
    let clean = relp.trim().trim_start_matches(['/', '\\']);
    if clean.split(['/', '\\']).any(|c| c == "..") {
        return Err("Caminho inválido.".into());
    }
    Ok(root.join(clean))
}

pub fn read(relp: &str) -> Result<String, String> {
    let root = vault();
    let mut p = inside(&root, relp)?;
    if !p.exists() && !relp.to_lowercase().ends_with(".md") {
        p = inside(&root, &format!("{relp}.md"))?;
    }
    if !p.exists() {
        // A title instead of a path.
        let hit = search(relp, 1)?.into_iter().next().ok_or_else(|| format!("Nota não encontrada: {relp}"))?;
        p = inside(&root, &hit.path)?;
    }
    let mut text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    if text.chars().count() > 15000 {
        text = text.chars().take(15000).collect::<String>() + "\n[…]";
    }
    Ok(text)
}

/// Safe file name from a title.
pub fn file_name(title: &str) -> String {
    let s: String = title.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Nota".into() } else { s }
}

/// Adds `text` to the note `title` (in `folder`, default "Jarvis"),
/// creating it when missing. Returns the vault-relative path.
pub fn write(title: &str, text: &str, folder: Option<&str>) -> Result<String, String> {
    let root = vault();
    if !root.is_dir() {
        return Err(format!("Vault do Obsidian não encontrado: {}", root.display()));
    }
    // An existing note with that title anywhere in the vault wins.
    let existing = {
        let mut files = Vec::new();
        walk(&root, &mut files, 0);
        let want = fold(title.trim());
        files.into_iter().find(|f| f.file_stem().map(|s| fold(&s.to_string_lossy()) == want).unwrap_or(false))
    };
    let path = match existing {
        Some(p) => p,
        None => {
            let dir = inside(&root, folder.filter(|f| !f.trim().is_empty()).unwrap_or("Jarvis"))?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            dir.join(format!("{}.md", file_name(title)))
        }
    };
    let now = chrono::Local::now().format("%d/%m/%Y %H:%M");
    let mut body = std::fs::read_to_string(&path).unwrap_or_default();
    if body.is_empty() {
        body = format!("# {}\n", title.trim());
    }
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("\n- {now} — {}\n", text.trim()));
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(rel(&root, &path))
}

pub fn describe(hits: &[NoteHit]) -> String {
    if hits.is_empty() {
        return "Nenhuma nota encontrada.".into();
    }
    hits.iter().map(|h| format!("{} — {}", h.path, h.snippet)).collect::<Vec<_>>().join("\n")
}

#[tauri::command]
pub async fn notes_search(query: String) -> Result<Vec<NoteHit>, String> {
    tauri::async_runtime::spawn_blocking(move || search(&query, 40)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn notes_read(path: String) -> Result<String, String> {
    read(&path)
}

#[tauri::command]
pub fn notes_write(title: String, text: String, folder: Option<String>) -> Result<String, String> {
    write(&title, &text, folder.as_deref())
}

/// Opens the note in Obsidian (obsidian:// link).
#[tauri::command]
pub fn notes_open(path: String) -> Result<String, String> {
    let root = vault();
    let vault_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut url = reqwest::Url::parse("obsidian://open").map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("vault", &vault_name).append_pair("file", &path);
    crate::jarvis_pc::jarvis_open_target(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(file_name("Ideias: app/jarvis?"), "Ideias- app-jarvis-");
        assert_eq!(file_name("  "), "Nota");
    }

    #[test]
    fn paths_cannot_leave_the_vault() {
        let root = Path::new("C:/v");
        assert!(inside(root, "../segredos.md").is_err());
        assert!(inside(root, "projetos/jarvis.md").is_ok());
    }

    #[test]
    fn snippets_show_the_matching_line() {
        let t = "# Jarvis\n\nOutra linha\nNotch com clima e FPS\n";
        assert_eq!(snippet(t, &["fps".into()]), "Notch com clima e FPS");
    }
}
