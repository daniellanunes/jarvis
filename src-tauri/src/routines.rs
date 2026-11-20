//! Routines: run their steps, and start them on their own when their trigger
//! fires — an app opening ("app:valorant") or a time of day ("time:18:00").
//!
//! Steps (a routine's `targets`):
//! - `close:chrome`        closes that program (gracefully)
//! - `spotify:lofi focus`  plays it on Spotify
//! - `volume:30`           sets the volume
//! - `saida:fone`          switches the audio output
//! - `mic:off` / `mic:on`  mutes / unmutes the microphones
//! - `say:texto`           Jarvis says it in the notch
//! - anything else         opened as usual (app, site, folder)

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::jarvis_store::{self, RoutineItem};

/// "app:Valorant" → ("app", "valorant"); "" → None.
pub fn parse_trigger(t: &str) -> Option<(&'static str, String)> {
    let (kind, value) = t.trim().split_once(':')?;
    let value = value.trim().to_lowercase();
    if value.is_empty() {
        return None;
    }
    match kind.trim().to_lowercase().as_str() {
        "app" => Some(("app", value.trim_end_matches(".exe").to_string())),
        "time" | "hora" => {
            let (h, m) = value.split_once(':')?;
            let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
            (h < 24 && m < 60).then(|| ("time", format!("{h:02}:{m:02}")))
        }
        _ => None,
    }
}

/// One step → (kind, value).
pub fn parse_step(step: &str) -> (&'static str, String) {
    let s = step.trim();
    if let Some((k, v)) = s.split_once(':') {
        let v = v.trim().to_string();
        match k.trim().to_lowercase().as_str() {
            "close" | "fechar" => return ("close", v),
            "spotify" | "tocar" => return ("spotify", v),
            "volume" => return ("volume", v),
            "saida" | "saída" | "output" => return ("output", v),
            "mic" | "microfone" => return ("mic", v),
            "say" | "falar" | "dizer" => return ("say", v),
            _ => {}
        }
    }
    ("open", s.to_string())
}

fn close_program(name: &str) -> Result<String, String> {
    let exe = if name.to_lowercase().ends_with(".exe") { name.to_string() } else { format!("{name}.exe") };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("taskkill")
            .args(["/IM", &exe])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(format!("{name} fechado"));
        }
        return Err(format!("{name} não estava aberto"));
    }
    #[cfg(not(windows))]
    Err(format!("Não sei fechar {exe} aqui"))
}

/// Runs every step; returns one line per step.
pub async fn run(app: &AppHandle, rt: &RoutineItem) -> Vec<String> {
    let mut out = Vec::new();
    for t in &rt.targets {
        let (kind, v) = parse_step(t);
        let r: Result<String, String> = match kind {
            "close" => close_program(&v),
            "spotify" => crate::spotify::resolve_and_play_spotify(app.clone(), v.clone(), None).await,
            "volume" => v.trim_end_matches('%').parse::<u32>().map_err(|_| format!("Volume inválido: {v}")).and_then(crate::jarvis_pc::jarvis_set_volume),
            "output" => {
                let v2 = v.clone();
                tauri::async_runtime::spawn_blocking(move || crate::audio::switch_output(&v2)).await.unwrap_or_else(|e| Err(e.to_string()))
            }
            "mic" => {
                let mute = matches!(v.to_lowercase().as_str(), "off" | "mudo" | "desligar" | "mute");
                tauri::async_runtime::spawn_blocking(move || crate::audio::set_mic_mute(mute))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                    .map(|_| if mute { "Microfone mudo".into() } else { "Microfone ligado".into() })
            }
            "say" => {
                let _ = app.emit_to(crate::island::WINDOW_LABEL, "jarvis-say", v.clone());
                Ok(v.clone())
            }
            _ => crate::jarvis_pc::jarvis_open_target(v.clone()),
        };
        out.push(r.unwrap_or_else(|e| format!("Erro: {e}")));
    }
    out
}

pub async fn run_named(app: &AppHandle, name: &str) -> Result<String, String> {
    let data = jarvis_store::load_data();
    let rt = data
        .routines
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name.trim()))
        .cloned()
        .ok_or_else(|| format!("Rotina não encontrada: {name}"))?;
    let lines = run(app, &rt).await;
    Ok(format!("Executando rotina: {}{}", rt.name, if lines.is_empty() { String::new() } else { format!(" — {}", lines.join("; ")) }))
}

#[tauri::command]
pub async fn jarvis_run_routine(app: AppHandle, name: String) -> Result<String, String> {
    run_named(&app, &name).await
}

#[cfg(windows)]
fn running_programs() -> HashSet<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut set = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return set };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            set.insert(name.trim_end_matches(".exe").to_string());
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    set
}

#[cfg(not(windows))]
fn running_programs() -> HashSet<String> {
    HashSet::new()
}

/// Matches "valorant" against "valorant-win64-shipping" etc.
pub fn app_matches(trigger: &str, running: &str) -> bool {
    running == trigger || running.starts_with(&format!("{trigger}-")) || running.starts_with(&format!("{trigger}_")) || running.starts_with(trigger) && trigger.len() >= 5
}

/// Checks the triggers every 3 s.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut before = running_programs();
        let mut ran_today: HashMap<String, String> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let routines: Vec<RoutineItem> = jarvis_store::load_data().routines.into_iter().filter(|r| parse_trigger(&r.trigger).is_some()).collect();
            if routines.is_empty() {
                before.clear();
                continue;
            }
            let now = running_programs();
            let started: Vec<&String> = if before.is_empty() { Vec::new() } else { now.iter().filter(|p| !before.contains(*p)).collect() };
            let clock = chrono::Local::now();
            let (today, hm) = (clock.format("%Y-%m-%d").to_string(), clock.format("%H:%M").to_string());
            for rt in routines {
                let Some((kind, value)) = parse_trigger(&rt.trigger) else { continue };
                let fire = match kind {
                    "app" => started.iter().any(|p| app_matches(&value, p)),
                    "time" => value == hm && ran_today.get(&rt.id) != Some(&today),
                    _ => false,
                };
                if fire {
                    ran_today.insert(rt.id.clone(), today.clone());
                    let app2 = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let lines = run(&app2, &rt).await;
                        crate::log::line(format!("routine {} by trigger: {}", rt.name, lines.join("; ")));
                        let _ = app2.emit_to(crate::island::WINDOW_LABEL, "routine-ran", format!("Rotina {}: {}", rt.name, lines.join(" · ")));
                    });
                }
            }
            before = now;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_parse() {
        assert_eq!(parse_trigger("app:Valorant.exe"), Some(("app", "valorant".into())));
        assert_eq!(parse_trigger("time:8:05"), Some(("time", "08:05".into())));
        assert_eq!(parse_trigger("time:25:00"), None);
        assert_eq!(parse_trigger(""), None);
    }

    #[test]
    fn steps_parse() {
        assert_eq!(parse_step("close:chrome"), ("close", "chrome".into()));
        assert_eq!(parse_step("spotify: lofi"), ("spotify", "lofi".into()));
        assert_eq!(parse_step("https://x.com"), ("open", "https://x.com".into()));
        assert_eq!(parse_step("notepad"), ("open", "notepad".into()));
    }

    #[test]
    fn game_processes_match_their_short_name() {
        assert!(app_matches("valorant", "valorant-win64-shipping"));
        assert!(app_matches("chrome", "chrome"));
        assert!(!app_matches("code", "codex"));
    }
}
