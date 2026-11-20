//! In-app updates. GitHub Releases host `latest.json` (written by
//! scripts/release.mjs) with the new installer and its signature; the
//! updater plugin checks it, refuses anything not signed with the project's
//! key, downloads, runs the installer and the app restarts on the new version.
//!
//! The island hears `update-available` (on start and every 6 h) and
//! `update-progress` while downloading.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    /// None when already on the latest version.
    pub version: Option<String>,
    pub notes: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: u64,
    phase: &'static str,
}

async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| format!("Não consegui verificar atualizações: {e}"))?;
    Ok(match found {
        Some(u) => UpdateInfo {
            current,
            version: Some(u.version.clone()),
            notes: u.body.clone().unwrap_or_default(),
            date: u.date.map(|d| d.to_string()).unwrap_or_default(),
        },
        None => UpdateInfo { current, version: None, notes: String::new(), date: String::new() },
    })
}

/// The current version and, if there is one, the newer one.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateInfo, String> {
    check(&app).await
}

/// Downloads, checks the signature and installs the new version. The
/// installer closes Jarvis and opens the new one when done.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("A atualização já está em andamento.".into());
    }
    let result = async {
        let updater = app.updater().map_err(|e| e.to_string())?;
        let update = updater
            .check()
            .await
            .map_err(|e| format!("Não consegui verificar atualizações: {e}"))?
            .ok_or("Você já está na versão mais recente.")?;
        crate::log::line(format!("update: baixando {}", update.version));
        let mut downloaded: u64 = 0;
        let progress_app = app.clone();
        let done_app = app.clone();
        update
            .download_and_install(
                move |chunk, total| {
                    downloaded += chunk as u64;
                    let _ = progress_app.emit_to(
                        crate::island::WINDOW_LABEL,
                        "update-progress",
                        Progress { downloaded, total: total.unwrap_or(0), phase: "downloading" },
                    );
                },
                move || {
                    let _ = done_app.emit_to(crate::island::WINDOW_LABEL, "update-progress", Progress { downloaded: 0, total: 0, phase: "installing" });
                },
            )
            .await
            .map_err(|e| format!("A atualização falhou: {e}"))?;
        crate::log::line("update: instalada, reiniciando");
        Ok::<(), String>(())
    }
    .await;
    INSTALLING.store(false, Ordering::SeqCst);
    result?;
    // On Windows the installer already closes the app; elsewhere, restart.
    app.restart();
}

/// Checks a little after start and then every 6 hours; tells the island.
pub fn spawn(app: AppHandle) {
    if cfg!(debug_assertions) {
        return; // `tauri dev` builds are not updated from GitHub
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            match check(&app).await {
                Ok(info) if info.version.is_some() => {
                    crate::log::line(format!("update: nova versão {} disponível", info.version.as_deref().unwrap_or("")));
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, "update-available", info);
                }
                Ok(_) => {}
                Err(e) => crate::log::line(format!("update: {e}")),
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}
