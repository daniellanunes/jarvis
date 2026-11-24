//! Local persistence & Obsidian Neural Mesh for Jarvis:
//! Memory nodes (user, project, tech, person, fact, pref), graph edges,
//! automatic workspace project scanner (`jarvis_config::projects_dir`), Tasks,
//! Reminders, Habits, Routines.
//!
//! Every read-modify-write goes through `STORE_LOCK`, saves are atomic
//! (temp file + rename) and keep a `.bak` of the previous version, and a file
//! that fails to parse is set aside instead of being overwritten.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::jarvis_config;

static STORE_LOCK: Mutex<()> = Mutex::new(());
static ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn lock_store() -> MutexGuard<'static, ()> {
    STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// An id unique within this process even when several are made in the same second.
fn unique_id(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{millis}_{seq}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryNode {
    pub id: String,
    pub label: String,
    pub category: String, // "user" | "project" | "tech" | "person" | "fact" | "pref"
    pub notes: String,
    pub created_at: u64,
}

impl Default for MemoryNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            category: "fact".into(),
            notes: String::new(),
            created_at: now_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub project: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ReminderItem {
    pub id: String,
    pub title: String,
    pub time: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HabitItem {
    pub id: String,
    pub name: String,
    pub streak: u32,
    pub done_today: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RoutineItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub targets: Vec<String>,
    /// "" (manual), "app:valorant" or "time:18:00" — see routines.rs.
    pub trigger: String,
}

/// "Carol" → "Carolina Braga": what the user calls a WhatsApp contact.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ContactNickname {
    pub nick: String,
    pub contact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JarvisData {
    pub user_name: String,
    pub assistant_name: String,
    pub accent_color: String,
    pub mochi_color: String,
    pub mochi_outfit: String,
    /// "solid" | "gradient" | "metallic" | "neon" (empty = solid).
    pub mochi_finish: String,
    pub level: u32,
    pub xp: u32,
    pub memories: Vec<MemoryNode>,
    pub edges: Vec<MemoryEdge>,
    pub tasks: Vec<TaskItem>,
    pub reminders: Vec<ReminderItem>,
    pub habits: Vec<HabitItem>,
    pub routines: Vec<RoutineItem>,
    pub contact_nicknames: Vec<ContactNickname>,
    /// WhatsApp contacts whose new messages Jarvis announces (and offers to read).
    pub whatsapp_watch: Vec<String>,
}

impl Default for JarvisData {
    fn default() -> Self {
        let user = jarvis_config::fallback_user_name();
        let mut d = Self {
            user_name: user.clone(),
            assistant_name: "Jarvis".into(),
            accent_color: "#84cc16".into(),
            mochi_color: "#f4f4f5".into(),
            mochi_outfit: "santaHat".into(),
            mochi_finish: "solid".into(),
            level: 1,
            xp: 35,
            memories: vec![
                MemoryNode {
                    id: "mem_root".into(),
                    label: user,
                    category: "user".into(),
                    notes: "Nó central da malha neural do Jarvis.".into(),
                    created_at: now_secs(),
                },
                MemoryNode {
                    id: "hub_projects".into(),
                    label: "Projetos".into(),
                    category: "pref".into(),
                    notes: format!("Repositórios e sistemas em {}", jarvis_config::projects_dir().display()),
                    created_at: now_secs(),
                },
                MemoryNode {
                    id: "hub_facts".into(),
                    label: "Outros fatos".into(),
                    category: "pref".into(),
                    notes: "Preferências, ideias e notas pessoais".into(),
                    created_at: now_secs(),
                },
            ],
            edges: vec![
                MemoryEdge {
                    from: "mem_root".into(),
                    to: "hub_projects".into(),
                    label: "desenvolve".into(),
                },
                MemoryEdge {
                    from: "mem_root".into(),
                    to: "hub_facts".into(),
                    label: "memórias".into(),
                },
            ],
            tasks: vec![
                TaskItem {
                    id: "tsk_1".into(),
                    title: "Evoluir malha neural do Jarvis".into(),
                    done: false,
                    project: "jarvis".into(),
                    created_at: now_secs(),
                },
            ],
            reminders: vec![],
            habits: vec![
                HabitItem {
                    id: "hab_1".into(),
                    name: "Beber água".into(),
                    streak: 3,
                    done_today: false,
                },
                HabitItem {
                    id: "hab_2".into(),
                    name: "Commitar nos projetos".into(),
                    streak: 5,
                    done_today: true,
                },
            ],
            routines: vec![
                RoutineItem {
                    id: "rot_work".into(),
                    name: "Modo Dev".into(),
                    description: "Abre VS Code, Terminal e Spotify".into(),
                    targets: vec!["vscode".into(), "terminal".into(), "spotify".into()],
                    trigger: String::new(),
                },
                RoutineItem {
                    id: "rot_focus".into(),
                    name: "Modo Foco".into(),
                    description: "Abre Google e Bloco de Notas".into(),
                    targets: vec!["google".into(), "notepad".into()],
                    trigger: String::new(),
                },
            ],
            contact_nicknames: Vec::new(),
            whatsapp_watch: Vec::new(),
        };
        populate_projects_into(&mut d);
        d
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn data_path() -> PathBuf {
    crate::platform::config_dir().join("jarvis_data.json")
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

/// Parses the store, falling back to the `.bak` copy. A file that exists but
/// does not parse is renamed to `jarvis_data.corrupt-<ts>.json` so it is
/// never silently overwritten.
fn read_from(path: &Path) -> (JarvisData, bool) {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => {
            // Missing file: try the backup before starting fresh.
            if let Some(d) = fs::read_to_string(backup_path(path))
                .ok()
                .and_then(|r| serde_json::from_str::<JarvisData>(&r).ok())
            {
                return (d, true);
            }
            return (JarvisData::default(), true);
        }
    };
    match serde_json::from_str::<JarvisData>(&raw) {
        Ok(d) => (d, false),
        Err(err) => {
            let aside = path.with_file_name(format!("jarvis_data.corrupt-{}.json", now_secs()));
            let _ = fs::rename(path, &aside);
            // Tests corrupt files on purpose: keep them out of the user's log.
            #[cfg(test)]
            let _ = &err;
            #[cfg(not(test))]
            crate::log::line(format!(
                "jarvis_data.json did not parse ({err}); kept at {}",
                aside.display()
            ));
            let restored = fs::read_to_string(backup_path(path))
                .ok()
                .and_then(|r| serde_json::from_str::<JarvisData>(&r).ok());
            (restored.unwrap_or_default(), true)
        }
    }
}

/// Atomic write: temp file in the same folder, `.bak` of the old version, rename.
fn write_to(path: &Path, data: &JarvisData) -> Result<(), String> {
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let _ = fs::copy(path, backup_path(path));
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Loads the store; the caller must hold `STORE_LOCK`.
fn load_unlocked() -> JarvisData {
    let path = data_path();
    let (mut data, mut dirty) = read_from(&path);
    if let Some(name) = jarvis_config::user_name_override() {
        if data.user_name != name {
            data.user_name = name;
            dirty = true;
        }
    } else if data.user_name.is_empty() || data.user_name == "Chefe" {
        data.user_name = jarvis_config::fallback_user_name();
        dirty = true;
    }
    if data.assistant_name.is_empty() {
        data.assistant_name = "Jarvis".into();
    }
    if data.mochi_outfit.is_empty() {
        data.mochi_outfit = "santaHat".into();
    }
    if data.mochi_color.is_empty() {
        data.mochi_color = "#f4f4f5".into();
    }
    // Ensure root & hubs exist and projects + Obsidian vault are scanned at least once
    let has_projects = data.memories.iter().any(|m| m.category == "project");
    let has_obsidian = data.memories.iter().any(|m| m.id == "obs_index");
    if !has_projects || !has_obsidian {
        if !has_projects {
            populate_projects_into(&mut data);
        } else {
            sync_obsidian_into(&mut data);
        }
        dirty = true;
    }
    if dirty {
        let _ = save_unlocked(&data);
    }
    data
}

/// Saves the store; the caller must hold `STORE_LOCK`.
fn save_unlocked(data: &JarvisData) -> Result<(), String> {
    let dir = crate::platform::config_dir();
    crate::platform::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    write_to(&data_path(), data)
}

pub fn load_data() -> JarvisData {
    let _g = lock_store();
    load_unlocked()
}

pub fn save_data(data: &JarvisData) -> Result<(), String> {
    let _g = lock_store();
    save_unlocked(data)
}

/// Load → change → save under one lock, so concurrent tool calls and the UI
/// cannot overwrite each other's changes.
pub fn update_data<R>(f: impl FnOnce(&mut JarvisData) -> R) -> Result<R, String> {
    let _g = lock_store();
    let mut data = load_unlocked();
    let out = f(&mut data);
    save_unlocked(&data)?;
    Ok(out)
}

fn ensure_node(data: &mut JarvisData, id: &str, label: &str, category: &str, notes: &str) {
    if let Some(existing) = data.memories.iter_mut().find(|m| m.id == id) {
        if !notes.is_empty() && existing.notes.is_empty() {
            existing.notes = notes.to_string();
        }
        return;
    }
    data.memories.push(MemoryNode {
        id: id.to_string(),
        label: label.to_string(),
        category: category.to_string(),
        notes: notes.to_string(),
        created_at: now_secs(),
    });
}

fn ensure_edge(data: &mut JarvisData, from: &str, to: &str, label: &str) {
    if data.edges.iter().any(|e| (e.from == from && e.to == to) || (e.from == to && e.to == from)) {
        return;
    }
    data.edges.push(MemoryEdge {
        from: from.to_string(),
        to: to.to_string(),
        label: label.to_string(),
    });
}

/// Scans the projects folder and populates Project + Tech nodes and neural mesh edges.
pub fn populate_projects_into(data: &mut JarvisData) {
    let workspace = jarvis_config::projects_dir();
    ensure_node(data, "mem_root", &data.user_name.clone(), "user", "Nó central da malha neural");
    ensure_node(data, "hub_projects", "Projetos", "pref", &format!("Workspace {}", workspace.display()));
    ensure_node(data, "hub_facts", "Outros fatos", "pref", "Memórias e fatos gerais");
    ensure_edge(data, "mem_root", "hub_projects", "projetos");
    ensure_edge(data, "mem_root", "hub_facts", "fatos");

    let Ok(entries) = fs::read_dir(&workspace) else {
        return;
    };

    for entry in entries.flatten().take(28) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }

        let pkg_json = path.join("package.json");
        let cargo_toml = path.join("Cargo.toml");
        let tauri_cargo = path.join("src-tauri").join("Cargo.toml");

        let mut stack = Vec::new();
        let mut desc = String::new();

        if let Ok(raw) = fs::read_to_string(&pkg_json) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(d) = v["description"].as_str() {
                    desc = d.to_string();
                }
                let deps = format!("{} {}", v["dependencies"], v["devDependencies"]);
                if deps.contains("react-native") || deps.contains("expo") {
                    stack.push(("tech_rn", "React Native"));
                } else if deps.contains("\"react\"") {
                    stack.push(("tech_react", "React"));
                }
                if deps.contains("next") {
                    stack.push(("tech_next", "Next.js"));
                }
                if deps.contains("typescript") {
                    stack.push(("tech_ts", "TypeScript"));
                }
                if deps.contains("tauri") {
                    stack.push(("tech_tauri", "Tauri / Rust"));
                }
            }
        }
        if cargo_toml.exists() || tauri_cargo.exists() {
            if !stack.iter().any(|(id, _)| *id == "tech_tauri") {
                stack.push(("tech_tauri", "Tauri / Rust"));
            }
        }

        let stack_names: Vec<&str> = stack.iter().map(|(_, n)| *n).collect();
        let notes = if !desc.is_empty() {
            format!("Caminho: {}\nStack: {}\n{}", path.display(), stack_names.join(", "), desc)
        } else if !stack_names.is_empty() {
            format!("Caminho: {}\nStack: {}", path.display(), stack_names.join(", "))
        } else {
            format!("Projeto em {}", path.display())
        };

        let proj_id = format!("proj_{}", name.to_lowercase().replace(' ', "_"));
        ensure_node(data, &proj_id, name, "project", &notes);
        ensure_edge(data, "hub_projects", &proj_id, "projeto");

        for (tech_id, tech_label) in stack {
            ensure_node(data, tech_id, tech_label, "tech", &format!("Tecnologia usada nos projetos ({tech_label})"));
            ensure_edge(data, &proj_id, tech_id, "usa");
        }
    }

    sync_obsidian_into(data);
}

#[derive(Default)]
struct ObsidianProjectMeta {
    name: String,
    path: String,
    stack: Vec<String>,
    updated: String,
    summary: String,
}

fn parse_obsidian_project(stem: &str, raw: &str) -> ObsidianProjectMeta {
    let mut meta = ObsidianProjectMeta {
        name: stem.to_string(),
        ..Default::default()
    };

    let mut body = raw;
    if let Some(rest) = raw.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            body = &rest[end + 4..];
            for line in fm.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("projeto:") {
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    if !val.is_empty() {
                        meta.name = val.to_string();
                    }
                } else if let Some(v) = line.strip_prefix("caminho:") {
                    meta.path = v.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if let Some(v) = line.strip_prefix("atualizado:") {
                    meta.updated = v.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if let Some(v) = line.strip_prefix("stack:") {
                    let inner = v.trim().trim_start_matches('[').trim_end_matches(']');
                    meta.stack = inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }

    // Extract meaningful lines (prioritizing Notas/Decisões/Status/Próximos Passos, skipping README code blocks)
    let mut in_code = false;
    let mut in_readme_section = false;
    let mut lines_out = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if trimmed.starts_with("## Resumo do README") {
            in_readme_section = true;
            continue;
        }
        if trimmed.starts_with("## ") {
            in_readme_section = false;
            lines_out.push(trimmed.to_string());
            continue;
        }
        if in_readme_section || trimmed.is_empty() || trimmed.starts_with("# ") {
            continue;
        }
        if trimmed.starts_with("- **Caminho:**")
            || trimmed.starts_with("- **Origem:**")
            || trimmed.starts_with("- **Stack detectada:**")
            || trimmed.starts_with("- **Graphify:**")
        {
            continue;
        }
        lines_out.push(trimmed.to_string());
        if lines_out.join("\n").len() >= 600 {
            break;
        }
    }

    let mut summary = lines_out.join("\n");
    if summary.len() > 650 {
        let mut end = 650;
        while !summary.is_char_boundary(end) {
            end -= 1;
        }
        summary.truncate(end);
        summary.push_str("...");
    }
    meta.summary = summary;
    meta
}

fn tech_id_for_name(name: &str) -> Option<(&'static str, &'static str)> {
    let lower = name.to_lowercase();
    if lower.contains("rust") || lower.contains("tauri") {
        Some(("tech_tauri", "Tauri / Rust"))
    } else if lower.contains("react native") || lower.contains("expo") {
        Some(("tech_rn", "React Native"))
    } else if lower.contains("next") {
        Some(("tech_next", "Next.js"))
    } else if lower.contains("react") {
        Some(("tech_react", "React"))
    } else if lower.contains("typescript") {
        Some(("tech_ts", "TypeScript"))
    } else if lower.contains("python") {
        Some(("tech_py", "Python"))
    } else if lower.contains("supabase") {
        Some(("tech_supabase", "Supabase"))
    } else if lower.contains("firebase") {
        Some(("tech_firebase", "Firebase"))
    } else {
        None
    }
}

/// Reads `INDEX.md` and `projetos/*.md` from the Obsidian Unified Memory vault
/// and merges them into Jarvis's neural mesh (`MemoryNode` + `MemoryEdge`).
pub fn sync_obsidian_into(data: &mut JarvisData) -> usize {
    let vault_buf = jarvis_config::obsidian_vault_dir();
    let vault = vault_buf.as_path();
    if !vault.exists() {
        return 0;
    }

    ensure_node(data, "mem_root", &data.user_name.clone(), "user", "Nó central da malha neural");
    ensure_node(
        data,
        "hub_projects",
        "Projetos",
        "pref",
        &format!("Workspace {} & Obsidian", jarvis_config::projects_dir().display()),
    );
    ensure_node(data, "hub_facts", "Outros fatos", "pref", "Memórias e fatos gerais");
    ensure_edge(data, "mem_root", "hub_projects", "projetos");
    ensure_edge(data, "mem_root", "hub_facts", "fatos");

    let mut count = 0usize;

    // 1. INDEX.md
    let index_path = vault.join("INDEX.md");
    if let Ok(raw) = fs::read_to_string(&index_path) {
        let mut projects_listed = Vec::new();
        let mut updated_line = String::new();
        for line in raw.lines() {
            let t = line.trim();
            if t.starts_with("> Última sincronização:") {
                updated_line = t.trim_start_matches('>').trim().to_string();
            } else if t.starts_with("- [[projetos/") {
                if let Some(name) = t.split('|').nth(1).and_then(|s| s.strip_suffix("]]")) {
                    projects_listed.push(name.trim().to_string());
                }
            }
        }
        let notes = format!(
            "[Obsidian Vault] {}\nProjetos indexados ({}): {}",
            if updated_line.is_empty() { "Memória Unificada" } else { &updated_line },
            projects_listed.len(),
            projects_listed.join(", ")
        );
        if let Some(existing) = data.memories.iter_mut().find(|m| m.id == "obs_index") {
            existing.notes = notes;
        } else {
            ensure_node(data, "obs_index", "Obsidian INDEX", "pref", &notes);
        }
        ensure_edge(data, "hub_facts", "obs_index", "obsidian");
        count += 1;
    }

    // 2. projetos/*.md
    let projetos_dir = vault.join("projetos");
    if let Ok(entries) = fs::read_dir(&projetos_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };

            let meta = parse_obsidian_project(stem, &raw);
            let proj_id = format!("proj_{}", stem.to_lowercase().replace([' ', '-'], "_"));
            let alt_proj_id = format!("proj_{}", stem.to_lowercase().replace(' ', "_"));

            let mut parts = Vec::new();
            if !meta.path.is_empty() {
                parts.push(format!("Caminho: {}", meta.path));
            }
            if !meta.stack.is_empty() {
                parts.push(format!("Stack: {}", meta.stack.join(", ")));
            }
            if !meta.updated.is_empty() {
                parts.push(format!("Obsidian atualizado: {}", meta.updated));
            }
            if !meta.summary.is_empty() {
                parts.push(meta.summary.clone());
            }
            let notes = parts.join("\n");

            // Reuse existing workspace project node if present (either dash or underscore id)
            let target_id = if data.memories.iter().any(|m| m.id == alt_proj_id) {
                alt_proj_id
            } else {
                proj_id
            };

            if let Some(existing) = data.memories.iter_mut().find(|m| m.id == target_id) {
                if !notes.is_empty() {
                    existing.notes = notes;
                }
            } else {
                ensure_node(data, &target_id, &meta.name, "project", &notes);
            }
            ensure_edge(data, "hub_projects", &target_id, "projeto");

            for tech in &meta.stack {
                if let Some((tech_id, tech_label)) = tech_id_for_name(tech) {
                    ensure_node(
                        data,
                        tech_id,
                        tech_label,
                        "tech",
                        &format!("Tecnologia usada nos projetos ({tech_label})"),
                    );
                    ensure_edge(data, &target_id, tech_id, "usa");
                }
            }
            count += 1;
        }
    }

    count
}

/// Builds a fresh prompt-aware context snippet directly from `INDEX.md` and `projetos/*.md`
/// so `jarvis_brain` always has up-to-date Obsidian memory.
pub fn load_obsidian_context(prompt: &str) -> String {
    let vault_buf = jarvis_config::obsidian_vault_dir();
    let vault = vault_buf.as_path();
    if !vault.exists() {
        return String::new();
    }

    let prompt_lower = prompt.to_lowercase();
    let mut out = String::new();

    // Compact INDEX.md overview
    if let Ok(index_raw) = fs::read_to_string(vault.join("INDEX.md")) {
        let mut projs = Vec::new();
        for line in index_raw.lines() {
            let t = line.trim();
            if t.starts_with("- [[projetos/") {
                if let Some(name) = t.split('|').nth(1).and_then(|s| s.strip_suffix("]]")) {
                    projs.push(name.trim().to_string());
                }
            }
        }
        if !projs.is_empty() {
            out.push_str(&format!(
                "Projetos no Obsidian ({}): {}\n",
                projs.len(),
                projs.join(", ")
            ));
        }
    }

    // Detailed notes for projects mentioned in the prompt (up to 3)
    let projetos_dir = vault.join("projetos");
    if let Ok(entries) = fs::read_dir(&projetos_dir) {
        let mut matched = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let stem_lower = stem.to_lowercase();
            let normalized = stem_lower.replace(['-', '_'], " ");
            if prompt_lower.contains(&stem_lower) || prompt_lower.contains(&normalized) {
                if let Ok(raw) = fs::read_to_string(&path) {
                    let meta = parse_obsidian_project(stem, &raw);
                    let mut block = format!("\n### Projeto Obsidian: {} ({})", meta.name, meta.path);
                    if !meta.stack.is_empty() {
                        block.push_str(&format!("\nStack: {}", meta.stack.join(", ")));
                    }
                    if !meta.updated.is_empty() {
                        block.push_str(&format!(" | Atualizado: {}", meta.updated));
                    }
                    if !meta.summary.is_empty() {
                        block.push('\n');
                        block.push_str(&meta.summary);
                    }
                    matched.push(block);
                    if matched.len() >= 3 {
                        break;
                    }
                }
            }
        }
        for m in matched {
            out.push_str(&m);
            out.push('\n');
        }
    }

    out
}

pub fn add_memory_internal(label: &str, category: &str, notes: &str, connect_to: Option<&str>) {
    let clean = label.trim();
    if clean.is_empty() {
        return;
    }
    let _ = update_data(|data| {
        let existing_id = data
            .memories
            .iter()
            .find(|m| m.label.eq_ignore_ascii_case(clean))
            .map(|m| m.id.clone());

        let node_id = if let Some(id) = existing_id {
            if !notes.trim().is_empty() {
                if let Some(m) = data.memories.iter_mut().find(|m| m.id == id) {
                    m.notes = notes.trim().to_string();
                }
            }
            id
        } else {
            let id = unique_id("mem");
            data.memories.push(MemoryNode {
                id: id.clone(),
                label: clean.to_string(),
                category: category.to_string(),
                notes: notes.trim().to_string(),
                created_at: now_secs(),
            });
            id
        };

        // Connect to target node or default hub
        if let Some(target_name) = connect_to.filter(|s| !s.trim().is_empty()) {
            if let Some(target_id) = data
                .memories
                .iter()
                .find(|m| m.label.eq_ignore_ascii_case(target_name.trim()) || m.id == target_name.trim())
                .map(|m| m.id.clone())
            {
                ensure_edge(data, &target_id, &node_id, "relacionado");
            }
        } else {
            let hub = if category == "project" { "hub_projects" } else { "hub_facts" };
            ensure_edge(data, hub, &node_id, category);
        }

        data.xp += 10;
        if data.xp >= data.level * 100 {
            data.xp -= data.level * 100;
            data.level += 1;
        }
    });
}

pub fn add_task_internal(title: &str) {
    let clean = title.trim();
    if clean.is_empty() {
        return;
    }
    let _ = update_data(|data| {
        data.tasks.push(TaskItem {
            id: unique_id("tsk"),
            title: clean.to_string(),
            done: false,
            project: String::new(),
            created_at: now_secs(),
        });
        data.xp += 5;
    });
}

pub fn add_reminder_internal(title: &str, time: &str) {
    let clean = title.trim();
    if clean.is_empty() {
        return;
    }
    let _ = update_data(|data| {
        data.reminders.push(ReminderItem {
            id: unique_id("rem"),
            title: clean.to_string(),
            time: if time.trim().is_empty() { "09:00".into() } else { time.trim().to_string() },
            done: false,
        });
        data.xp += 5;
    });
}

/// Saves (or changes) a contact nickname.
pub fn set_nickname(nick: &str, contact: &str) -> Result<(), String> {
    let (nick, contact) = (nick.trim(), contact.trim());
    if nick.is_empty() || contact.is_empty() {
        return Err("Falta o apelido ou o contato.".into());
    }
    let key = crate::file_search::fold(nick);
    update_data(|d| {
        d.contact_nicknames.retain(|n| crate::file_search::fold(&n.nick) != key);
        d.contact_nicknames.push(ContactNickname { nick: nick.to_string(), contact: contact.to_string() });
    })
}

/// Adds or removes a watched WhatsApp contact. True when the list changed.
pub fn set_watch(contact: &str, on: bool) -> Result<bool, String> {
    let c = contact.trim();
    if c.is_empty() {
        return Err("Qual contato?".into());
    }
    let key = crate::file_search::fold(c);
    update_data(|d| {
        let had = d.whatsapp_watch.iter().any(|w| crate::file_search::fold(w) == key);
        d.whatsapp_watch.retain(|w| crate::file_search::fold(w) != key);
        if on {
            d.whatsapp_watch.push(c.to_string());
        }
        had != on
    })
}

/// Removes a nickname. True when there was one.
pub fn remove_nickname(nick: &str) -> Result<bool, String> {
    let key = crate::file_search::fold(nick.trim());
    update_data(|d| {
        let before = d.contact_nicknames.len();
        d.contact_nicknames.retain(|n| crate::file_search::fold(&n.nick) != key);
        before != d.contact_nicknames.len()
    })
}

pub fn add_routine_internal(name: &str, description: &str, targets: Vec<String>) {
    let clean = name.trim();
    if clean.is_empty() {
        return;
    }
    let _ = update_data(|data| {
        if let Some(existing) = data.routines.iter_mut().find(|r| r.name.eq_ignore_ascii_case(clean)) {
            existing.description = description.to_string();
            existing.targets = targets;
        } else {
            data.routines.push(RoutineItem {
                id: unique_id("rot"),
                name: clean.to_string(),
                description: description.to_string(),
                targets,
                trigger: String::new(),
            });
        }
        data.xp += 15;
    });
}

#[tauri::command]
pub fn jarvis_data_load() -> JarvisData {
    load_data()
}

#[tauri::command]
pub fn jarvis_data_save(data: JarvisData) -> Result<(), String> {
    save_data(&data)
}

#[tauri::command]
pub fn jarvis_scan_projects() -> Result<JarvisData, String> {
    update_data(|data| {
        populate_projects_into(data);
        data.clone()
    })
}

#[tauri::command]
pub fn jarvis_sync_obsidian() -> Result<JarvisData, String> {
    update_data(|data| {
        sync_obsidian_into(data);
        data.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jarvis-store-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("jarvis_data.json")
    }

    fn sample(name: &str) -> JarvisData {
        JarvisData {
            user_name: name.into(),
            memories: vec![],
            edges: vec![],
            tasks: vec![],
            ..JarvisData::default()
        }
    }

    #[test]
    fn unique_ids_do_not_repeat_within_the_same_second() {
        let a = unique_id("mem");
        let b = unique_id("mem");
        assert_ne!(a, b);
        assert!(a.starts_with("mem_"));
    }

    #[test]
    fn a_save_keeps_the_previous_version_as_backup() {
        let path = scratch("bak");
        write_to(&path, &sample("first")).unwrap();
        write_to(&path, &sample("second")).unwrap();
        let (now, _) = read_from(&path);
        assert_eq!(now.user_name, "second");
        let bak: JarvisData = serde_json::from_str(&fs::read_to_string(backup_path(&path)).unwrap()).unwrap();
        assert_eq!(bak.user_name, "first");
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_the_backup_restored() {
        let path = scratch("corrupt");
        write_to(&path, &sample("good")).unwrap();
        write_to(&path, &sample("good")).unwrap(); // .bak now holds "good"
        fs::write(&path, b"{ not json").unwrap();
        let (data, dirty) = read_from(&path);
        assert!(dirty);
        assert_eq!(data.user_name, "good");
        let aside = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("jarvis_data.corrupt-"));
        assert!(aside, "the unreadable file must be kept, not overwritten");
    }
}
