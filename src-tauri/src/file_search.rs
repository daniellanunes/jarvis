//! "Acha o PDF do contrato de março": a name index of the user's folders
//! (Desktop, Documents, Downloads, Pictures, Videos, Music, OneDrive and the
//! projects folder), built in the background on first use and refreshed every
//! 15 minutes. Searches are in memory, accent- and case-insensitive.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: u64,
}

struct Entry {
    path: PathBuf,
    key: String,
    is_dir: bool,
    size: u64,
    modified: u64,
}

struct Index {
    entries: Vec<Entry>,
    built: Instant,
}

static INDEX: LazyLock<Mutex<Option<Index>>> = LazyLock::new(|| Mutex::new(None));
static BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const MAX_ENTRIES: usize = 400_000;
const REFRESH: Duration = Duration::from_secs(15 * 60);

/// Lowercase without accents: "Março.pdf" → "marco.pdf".
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// Folders that are never worth searching (and can hold millions of files).
pub fn skipped_dir(name: &str) -> bool {
    const SKIP: &[&str] = &[
        "node_modules", ".git", "target", ".venv", "venv", "__pycache__", ".next", "dist", "build", ".gradle", ".idea", ".vs",
        "$recycle.bin", "appdata", ".cache", ".npm", ".cargo", ".rustup", "site-packages", "obj", "bin", ".svn", "pods",
        // Generated knowledge-graph exports: thousands of code-symbol notes.
        "graphify", "graphify-out",
    ];
    let n = name.to_ascii_lowercase();
    n.starts_with('.') && n != ".obsidian" && n.len() > 1 && SKIP.contains(&n.as_str()) || SKIP.contains(&n.as_str())
}

fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        for d in ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music", "OneDrive", "Área de Trabalho", "Documentos", "Imagens"] {
            let p = home.join(d);
            if p.is_dir() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    let p = crate::jarvis_config::projects_dir();
    {
        if p.is_dir() && !out.iter().any(|r| p.starts_with(r)) {
            out.push(p);
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    if depth > 12 || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let is_dir = ft.is_dir();
        if is_dir && skipped_dir(&name) {
            continue;
        }
        let meta = e.metadata().ok();
        out.push(Entry {
            key: fold(&name),
            path: e.path(),
            is_dir,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
        if is_dir {
            walk(&e.path(), depth + 1, out);
        }
    }
}

fn build() {
    if BUILDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let mut entries = Vec::new();
    for r in roots() {
        walk(&r, 0, &mut entries);
    }
    *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Index { entries, built: Instant::now() });
    BUILDING.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Score of a name for the query words: all words must be in the name (or
/// for the last ones, in the folder path); exact extension/prefix rank higher.
pub fn score(name_key: &str, path_key: &str, words: &[String]) -> Option<i32> {
    let mut s = 0;
    for w in words {
        if let Some(pos) = name_key.find(w.as_str()) {
            s += 10 + if pos == 0 { 6 } else { 0 } + if name_key.ends_with(&format!(".{w}")) { 4 } else { 0 };
        } else if path_key.contains(w.as_str()) {
            s += 3;
        } else {
            return None;
        }
    }
    Some(s)
}

/// Word aliases: "pdf"/"planilha"/"foto" stand for extensions.
fn expand(word: &str) -> String {
    match word {
        "planilha" | "planilhas" => "xls".into(),
        "foto" | "fotos" | "imagem" | "imagens" => "".into(),
        "documento" | "documentos" => "".into(),
        "arquivo" | "arquivos" | "o" | "a" | "de" | "do" | "da" | "um" | "uma" | "meu" | "minha" => "".into(),
        w => w.into(),
    }
}

pub fn search(query: &str, limit: usize) -> Result<Vec<FileHit>, String> {
    let fresh = INDEX.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|i| i.built.elapsed() < REFRESH);
    match fresh {
        None => build(),
        Some(false) => {
            std::thread::spawn(build);
        }
        _ => {}
    }
    let words: Vec<String> = fold(query).split_whitespace().map(expand).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    let Some(idx) = guard.as_ref() else { return Err("O índice de arquivos ainda está sendo montado. Tente de novo em alguns segundos.".into()) };
    let mut hits: Vec<(i32, &Entry)> = idx
        .entries
        .iter()
        .filter_map(|e| {
            let parent = e.path.parent().map(|p| fold(&p.to_string_lossy())).unwrap_or_default();
            score(&e.key, &parent, &words).map(|s| (s, e))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.modified.cmp(&a.1.modified)));
    Ok(hits
        .into_iter()
        .take(limit)
        .map(|(_, e)| FileHit {
            path: e.path.to_string_lossy().to_string(),
            name: e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            dir: e.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            is_dir: e.is_dir,
            size: e.size,
            modified: e.modified,
        })
        .collect())
}

pub fn describe(hits: &[FileHit]) -> String {
    if hits.is_empty() {
        return "Nenhum arquivo encontrado.".into();
    }
    hits.iter().map(|h| format!("{}{}", h.path, if h.is_dir { " (pasta)" } else { "" })).collect::<Vec<_>>().join("\n")
}

/// Opens a file (default app) or shows it selected in Explorer.
pub fn open(path: &str, reveal: bool) -> Result<String, String> {
    if !Path::new(path).exists() {
        return Err(format!("Arquivo não existe mais: {path}"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer");
        if reveal {
            cmd.arg(format!("/select,{path}"));
        } else {
            cmd.arg(path);
        }
        cmd.creation_flags(0x0800_0000).spawn().map_err(|e| e.to_string())?;
    }
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(if reveal { format!("Mostrando {name} na pasta") } else { format!("Abrindo {name}") })
}

#[tauri::command]
pub async fn files_search(query: String) -> Result<Vec<FileHit>, String> {
    tauri::async_runtime::spawn_blocking(move || search(&query, 60)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn files_open(path: String, reveal: bool) -> Result<String, String> {
    open(&path, reveal)
}

/// Starts building the index early (when the Arquivos tab opens).
#[tauri::command]
pub fn files_warm() {
    if INDEX.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        std::thread::spawn(build);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fold_accents() {
        assert_eq!(fold("Contrato Março.PDF"), "contrato marco.pdf");
    }

    #[test]
    fn every_word_must_match() {
        let w = |q: &str| fold(q).split_whitespace().map(expand).filter(|w| !w.is_empty()).collect::<Vec<_>>();
        assert!(score("contrato marco.pdf", "c:\\docs", &w("pdf do contrato de março")).is_some());
        assert!(score("contrato abril.pdf", "c:\\docs", &w("contrato março")).is_none());
        // A word found in the folder still matches, lower.
        let a = score("contrato.pdf", "c:\\docs\\marco", &w("contrato março")).unwrap();
        let b = score("contrato marco.pdf", "c:\\docs", &w("contrato março")).unwrap();
        assert!(b > a);
    }

    #[test]
    fn heavy_folders_are_skipped() {
        assert!(skipped_dir("node_modules"));
        assert!(skipped_dir(".git"));
        assert!(!skipped_dir("Contratos"));
    }
}
