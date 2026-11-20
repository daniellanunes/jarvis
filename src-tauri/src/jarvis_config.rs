//! Jarvis paths and identities that used to be hardcoded (Obsidian vault,
//! projects folder, user name, OpenBot webhook). Each value is resolved as:
//! the setting (Settings → Jarvis) → an environment variable → a default
//! derived from the machine, never from a specific user's name.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use crate::settings::Settings;

/// The OpenBot endpoint used when neither the setting nor
/// `OPENBOT_WEBHOOK_URL` gives one.
const DEFAULT_OPENBOT_WEBHOOK_URL: &str =
    "https://signal.openbot.run/v1/webhooks/f0769015-f6df-4914-b1bd-1790fdf79f0d";

#[derive(Debug, Clone, Default)]
struct Overrides {
    obsidian_vault: String,
    projects_dir: String,
    user_name: String,
    openbot_webhook_url: String,
    confirm_system_actions: bool,
    spotify_client_id: String,
}

static OVERRIDES: LazyLock<RwLock<Overrides>> = LazyLock::new(|| RwLock::new(Overrides::default()));

/// Called at startup and after every settings save.
pub fn apply_settings(s: &Settings) {
    let mut o = OVERRIDES.write().unwrap_or_else(|e| e.into_inner());
    o.obsidian_vault = s.jarvis_obsidian_vault.trim().to_string();
    o.projects_dir = s.jarvis_projects_dir.trim().to_string();
    o.user_name = s.jarvis_user_name.trim().to_string();
    o.openbot_webhook_url = s.jarvis_openbot_webhook_url.trim().to_string();
    o.confirm_system_actions = s.jarvis_confirm_system_actions;
    o.spotify_client_id = s.jarvis_spotify_client_id.trim().to_string();
}

/// Client ID of the user's own Spotify developer app (Web API, PKCE).
pub fn spotify_client_id() -> Option<String> {
    pick(&read().spotify_client_id, "SPOTIFY_CLIENT_ID")
}

fn read() -> Overrides {
    OVERRIDES.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn pick(setting: &str, env: &str) -> Option<String> {
    if !setting.is_empty() {
        return Some(setting.to_string());
    }
    env_nonempty(env)
}

/// The Obsidian "Memória Unificada" folder (holds INDEX.md and projetos/).
pub fn obsidian_vault_dir() -> PathBuf {
    if let Some(p) = pick(&read().obsidian_vault, "JARVIS_OBSIDIAN_VAULT") {
        return PathBuf::from(p);
    }
    home_dir()
        .unwrap_or_default()
        .join("Documents")
        .join("Obsidian Vault")
        .join("Memoria-Unificada")
}

/// The workspace folder scanned for projects.
pub fn projects_dir() -> PathBuf {
    if let Some(p) = pick(&read().projects_dir, "JARVIS_PROJECTS_DIR") {
        return PathBuf::from(p);
    }
    let d_drive = PathBuf::from(r"D:\Projetos");
    if d_drive.is_dir() {
        return d_drive;
    }
    home_dir().unwrap_or_default().join("Projetos")
}

/// The name Jarvis calls the user, or None to keep whatever the store has.
pub fn user_name_override() -> Option<String> {
    pick(&read().user_name, "JARVIS_USER_NAME")
}

/// A name for a brand-new store: the OS account name, else a neutral word.
pub fn fallback_user_name() -> String {
    user_name_override()
        .or_else(|| env_nonempty("USERNAME"))
        .or_else(|| env_nonempty("USER"))
        .unwrap_or_else(|| "Usuário".to_string())
}

pub fn openbot_webhook_url() -> String {
    pick(&read().openbot_webhook_url, "OPENBOT_WEBHOOK_URL")
        .unwrap_or_else(|| DEFAULT_OPENBOT_WEBHOOK_URL.to_string())
}

/// When on, PowerShell/CMD actions wait for Permitir/Cancelar in the island.
/// Off by default: the current behaviour (direct execution) is kept.
pub fn confirm_system_actions() -> bool {
    read().confirm_system_actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_win_over_defaults() {
        let mut s = Settings::default();
        s.jarvis_obsidian_vault = r"X:\Vault".into();
        s.jarvis_projects_dir = r"X:\Code".into();
        s.jarvis_user_name = "Ana".into();
        s.jarvis_openbot_webhook_url = "https://example.test/hook".into();
        s.jarvis_confirm_system_actions = true;
        apply_settings(&s);
        assert_eq!(obsidian_vault_dir(), PathBuf::from(r"X:\Vault"));
        assert_eq!(projects_dir(), PathBuf::from(r"X:\Code"));
        assert_eq!(user_name_override().as_deref(), Some("Ana"));
        assert_eq!(openbot_webhook_url(), "https://example.test/hook");
        assert!(confirm_system_actions());
        apply_settings(&Settings::default());
        assert!(!confirm_system_actions());
        assert!(obsidian_vault_dir().ends_with("Memoria-Unificada"));
    }
}
