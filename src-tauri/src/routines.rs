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
//! - `esperar:5`           waits 5 seconds (an app that takes a while to open)
//! - `teclas:ctrl+shift+n` presses keys in the window in front
//! - `digitar:texto`       types it in the window in front
//! - `obs:gravar`          OBS: gravar, parar, live, parar live, cena X, mutar…
//! - `projeto:nome`        opens that project (in the projects folder) in the editor
//! - `whatsapp:Bia: texto` asks "Posso enviar?" before sending it
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
            "wait" | "esperar" | "aguardar" => return ("wait", v),
            "keys" | "teclas" | "atalho" => return ("keys", v),
            "type" | "digitar" | "escrever" => return ("type", v),
            "obs" => return ("obs", v),
            "projeto" | "project" => return ("project", v),
            "whatsapp" | "mensagem" => return ("whatsapp", v),
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

/// A project folder by name ("alugamobile", "aluga mobile").
pub fn find_project(name: &str) -> Option<String> {
    let want = crate::file_search::fold(name.trim()).replace([' ', '-', '_'], "");
    if want.is_empty() {
        return None;
    }
    let dirs: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(crate::jarvis_config::projects_dir())
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (crate::file_search::fold(&e.file_name().to_string_lossy()).replace([' ', '-', '_'], ""), e.path()))
        .collect();
    dirs.iter()
        .find(|(n, _)| *n == want)
        .or_else(|| dirs.iter().filter(|(n, _)| n.starts_with(&want)).min_by_key(|(n, _)| n.len()))
        .or_else(|| dirs.iter().filter(|(n, _)| n.contains(&want)).min_by_key(|(n, _)| n.len()))
        .map(|(_, p)| p.to_string_lossy().to_string())
}

/// Runs every step; returns one line per step.
pub async fn run(app: &AppHandle, rt: &RoutineItem) -> Vec<String> {
    let mut out = Vec::new();
    // The program the routine opened last: typing and keys go to it.
    let mut last_app: Option<String> = None;
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
            "wait" => match v.trim().trim_end_matches('s').replace(',', ".").parse::<f32>() {
                Ok(secs) => {
                    let secs = secs.clamp(0.0, 120.0);
                    tokio::time::sleep(Duration::from_millis((secs * 1000.0) as u64)).await;
                    Ok(format!("Esperei {secs} s"))
                }
                Err(_) => Err(format!("Tempo inválido: {v}")),
            },
            "keys" | "type" => {
                let (text, keys) = if kind == "type" { (v.clone(), String::new()) } else { (String::new(), v.clone()) };
                let target = last_app.clone();
                tauri::async_runtime::spawn_blocking(move || match target {
                    // In front first (it may still be opening), then the keys.
                    Some(app) => crate::pc_control::type_into(&app, &text, &keys, "")
                        .or_else(|_| if keys.is_empty() { crate::pc_control::type_here(&text) } else { crate::pc_control::press_here(&keys) }.map(|_| String::new())),
                    None => if keys.is_empty() { crate::pc_control::type_here(&text) } else { crate::pc_control::press_here(&keys) }.map(|_| String::new()),
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
                .map(|_| if kind == "type" { "Digitei".to_string() } else { format!("Apertei {v}") })
            }
            "obs" => crate::obs::step(&v).await,
            "project" => match find_project(&v) {
                Some(path) => crate::jarvis_pc::jarvis_open_target(format!("editor:{path}")),
                None => Err(format!("Projeto não encontrado: {v}")),
            },
            "whatsapp" => match v.split_once(':') {
                Some((to, msg)) if !to.trim().is_empty() && !msg.trim().is_empty() => Ok(crate::whatsapp::ask_to_send(app, to, msg)),
                _ => Err("Use whatsapp:Contato: mensagem".into()),
            },
            _ => {
                let r = crate::jarvis_pc::jarvis_open_target(v.clone());
                if r.is_ok() {
                    last_app = Some(v.clone());
                }
                r
            }
        };
        out.push(r.unwrap_or_else(|e| format!("Erro: {e}")));
    }
    out
}

/// Runs a routine and says, in one line, what did not work.
pub async fn run_spoken(app: &AppHandle, name: &str) -> String {
    let Some(rt) = jarvis_store::load_data().routines.into_iter().find(|r| r.name.eq_ignore_ascii_case(name.trim())) else {
        return format!("Rotina não encontrada: {name}");
    };
    let lines = run(app, &rt).await;
    let failed: Vec<String> = rt
        .targets
        .iter()
        .zip(&lines)
        .filter(|(_, l)| l.starts_with("Erro"))
        .map(|(t, l)| {
            crate::log::line(format!("rotina {}: {t} → {l}", rt.name));
            parse_step(t).1
        })
        .collect();
    if failed.is_empty() {
        format!("Rodando {}.", rt.name)
    } else {
        format!("Rodando {}, mas não consegui abrir: {}.", rt.name, failed.join(", "))
    }
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

/// "Modo trabalho", "ativa o modo trabalho", "roda a rotina modo trabalho":
/// the routine whose name was said (nothing else), to run at once.
pub fn spoken(said: &str) -> Option<String> {
    let clean = |s: &str| -> String {
        crate::file_search::fold(s).chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let mut t = clean(said);
    for _ in 0..3 {
        for pre in [
            "jarvis ", "por favor ", "ativa ", "ativar ", "ative ", "liga ", "ligar ", "roda ", "rodar ", "rode ", "executa ", "executar ", "execute ",
            "inicia ", "iniciar ", "inicie ", "comeca ", "começa ", "abre ", "abrir ", "abra ", "a rotina ", "rotina ", "o ", "a ", "meu ", "minha ",
        ] {
            if let Some(rest) = t.strip_prefix(pre) {
                t = rest.to_string();
            }
        }
    }
    for tail in [" por favor", " agora", " ai", " pra mim", " para mim", " jarvis"] {
        if let Some(rest) = t.strip_suffix(tail) {
            t = rest.to_string();
        }
    }
    if t.len() < 3 {
        return None;
    }
    jarvis_store::load_data().routines.into_iter().find(|r| clean(&r.name) == t || clean(&r.name).strip_prefix("rotina ") == Some(t.as_str())).map(|r| r.name)
}

#[tauri::command]
pub async fn jarvis_run_routine(app: AppHandle, name: String) -> Result<String, String> {
    run_named(&app, &name).await
}

#[cfg(windows)]
pub(crate) fn running_programs() -> HashSet<String> {
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
pub(crate) fn running_programs() -> HashSet<String> {
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

#[cfg(test)]
mod probe {
    use super::parse_step;
    /// With this PC's data: `cargo test --lib routines::probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn what_is_understood_here() {
        for t in ["obs", "OBS Studio", "antigravity", "obsidian", "programa que nao existe"] {
            println!("abre {t:?} → {:?}", crate::jarvis_pc::opens_what(t));
        }
        for r in crate::jarvis_store::load_data().routines {
            println!("rotina {:?}: {:?}", r.name, r.targets);
            for t in &r.targets {
                println!("   {t:?} → {:?}", crate::jarvis_pc::find_app(&parse_step(t).1));
            }
        }
        for said in ["Modo Dev", "Jarvis, ativa o modo dev, por favor.", "roda a rotina modo foco", "abre o spotify", "modo"] {
            println!("{said:?} → {:?}", super::spoken(said));
        }
    }
}


#[cfg(test)]
mod steps {
    use super::parse_step;

    #[test]
    fn every_step_kind_is_read() {
        assert_eq!(parse_step("esperar:5"), ("wait", "5".into()));
        assert_eq!(parse_step("teclas: ctrl+shift+n"), ("keys", "ctrl+shift+n".into()));
        assert_eq!(parse_step("digitar:Olá, tudo bem?"), ("type", "Olá, tudo bem?".into()));
        assert_eq!(parse_step("obs:cena Câmera"), ("obs", "cena Câmera".into()));
        assert_eq!(parse_step("projeto:alugamobile"), ("project", "alugamobile".into()));
        assert_eq!(parse_step("whatsapp:Bia: chego em 10 min"), ("whatsapp", "Bia: chego em 10 min".into()));
        assert_eq!(parse_step("https://youtube.com"), ("open", "https://youtube.com".into()));
        assert_eq!(parse_step(r"C:\Jogos\jogo.exe"), ("open", r"C:\Jogos\jogo.exe".into()));
        assert_eq!(parse_step("Antigravity IDE"), ("open", "Antigravity IDE".into()));
    }
}

#[cfg(all(test, windows))]
mod live_steps {
    /// Opens the Calculator, waits, types a sum, reads it, finds a project.
    /// `cargo test --lib routines::live_steps -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn open_wait_type_and_project() {
        // As a routine does it: open, then type and press into that program.
        crate::jarvis_pc::jarvis_open_target("calc".into()).unwrap();
        println!("{:?}", crate::pc_control::type_into("calc", "12*3=", "", ""));
        let r = crate::pc_control::read("calculadora").unwrap();
        println!("{}", r.lines().find(|l| l.contains("exibição")).unwrap_or("?"));
        println!("{:?}", crate::pc_control::type_into("calc", "", "alt+f4", ""));
        println!("projeto: {:?}", super::find_project("aluga mobile"));
    }
}
