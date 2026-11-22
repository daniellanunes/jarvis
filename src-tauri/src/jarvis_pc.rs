//! PC control actions for Jarvis: launching apps/URLs and media/volume keys.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start menu and Desktop shortcuts (.lnk / .url), by folded name. Rebuilt
/// every 5 minutes: installing an app shows up without a restart.
static SHORTCUTS: LazyLock<Mutex<Option<(std::time::Instant, Vec<(String, std::path::PathBuf)>)>>> = LazyLock::new(|| Mutex::new(None));

fn shortcut_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    for (var, sub) in [
        ("APPDATA", r"Microsoft\Windows\Start Menu\Programs"),
        ("PROGRAMDATA", r"Microsoft\Windows\Start Menu\Programs"),
        ("USERPROFILE", "Desktop"),
        ("PUBLIC", "Desktop"),
        ("USERPROFILE", r"OneDrive\Desktop"),
        ("USERPROFILE", r"OneDrive\Área de Trabalho"),
    ] {
        if let Ok(base) = std::env::var(var) {
            let p = std::path::Path::new(&base).join(sub);
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    roots
}

fn collect_shortcuts(dir: &std::path::Path, depth: usize, out: &mut Vec<(String, std::path::PathBuf)>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_shortcuts(&p, depth + 1, out);
            continue;
        }
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext == "lnk" || ext == "url" {
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            // Uninstallers and help files are never what "abre X" means.
            let f = crate::file_search::fold(&stem);
            if f.contains("uninstall") || f.contains("desinstal") || f.contains("readme") || f.contains("help") {
                continue;
            }
            out.push((f, p));
        }
    }
}

/// Words that are not part of an app's name ("abre o app do OpenBot").
fn app_query(name: &str) -> String {
    crate::file_search::fold(name)
        .split_whitespace()
        .filter(|w| !matches!(*w, "o" | "a" | "os" | "as" | "app" | "aplicativo" | "programa" | "do" | "da" | "de" | "meu" | "minha"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Best shortcut for an app name: exact name, then starts with, then contains.
pub fn find_app_shortcut(name: &str) -> Option<std::path::PathBuf> {
    let q = app_query(name);
    if q.len() < 2 {
        return None;
    }
    let mut guard = SHORTCUTS.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(300)) {
        let mut list = Vec::new();
        for r in shortcut_roots() {
            collect_shortcuts(&r, 0, &mut list);
        }
        *guard = Some((std::time::Instant::now(), list));
    }
    let list = &guard.as_ref()?.1;
    pick_shortcut(list, &q)
}

pub fn pick_shortcut(list: &[(String, std::path::PathBuf)], q: &str) -> Option<std::path::PathBuf> {
    let nospace = |s: &str| s.replace([' ', '-', '_', '.'], "");
    let qn = nospace(q);
    list.iter()
        .find(|(n, _)| n == q || nospace(n) == qn)
        .or_else(|| list.iter().filter(|(n, _)| n.starts_with(q) || nospace(n).starts_with(&qn)).min_by_key(|(n, _)| n.len()))
        .or_else(|| list.iter().filter(|(n, _)| q.len() >= 4 && (n.contains(q) || nospace(n).contains(&qn))).min_by_key(|(n, _)| n.len()))
        .map(|(_, p)| p.clone())
}

/// A code editor found on the PC.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorInfo {
    pub name: String,
    /// The program; also what Ajustes saves as the choice.
    pub path: String,
}

/// The code editors installed on this PC, in the order the automatic choice
/// prefers them. Looks in the usual install folders and on PATH.
pub fn installed_editors() -> Vec<EditorInfo> {
    use std::path::PathBuf;
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let local = env("LOCALAPPDATA").unwrap_or_default();
    let pf = env("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let pf86 = env("ProgramFiles(x86)").unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));

    // (name, install paths, command on PATH)
    let known: Vec<(&str, Vec<PathBuf>, &str)> = vec![
        ("Antigravity IDE", vec![local.join(r"Programs\Antigravity IDE\Antigravity IDE.exe")], "antigravity-ide"),
        ("Antigravity", vec![local.join(r"Programs\Antigravity\Antigravity.exe")], "antigravity"),
        ("Cursor", vec![local.join(r"Programs\cursor\Cursor.exe"), pf.join(r"Cursor\Cursor.exe")], "cursor"),
        ("Windsurf", vec![local.join(r"Programs\Windsurf\Windsurf.exe"), pf.join(r"Windsurf\Windsurf.exe")], "windsurf"),
        ("VS Code", vec![local.join(r"Programs\Microsoft VS Code\Code.exe"), pf.join(r"Microsoft VS Code\Code.exe")], "code"),
        (
            "VS Code Insiders",
            vec![local.join(r"Programs\Microsoft VS Code Insiders\Code - Insiders.exe"), pf.join(r"Microsoft VS Code Insiders\Code - Insiders.exe")],
            "code-insiders",
        ),
        ("Zed", vec![local.join(r"Programs\Zed\Zed.exe"), pf.join(r"Zed\Zed.exe")], "zed"),
        ("Sublime Text", vec![pf.join(r"Sublime Text\sublime_text.exe"), pf.join(r"Sublime Text 3\sublime_text.exe")], "subl"),
    ];
    let mut out: Vec<EditorInfo> = Vec::new();
    let push = |name: String, exe: PathBuf, out: &mut Vec<EditorInfo>| {
        let path = exe.to_string_lossy().to_string();
        if !out.iter().any(|e| e.path.eq_ignore_ascii_case(&path) || e.name == name) {
            out.push(EditorInfo { name, path });
        }
    };
    for (name, paths, cmd) in known {
        if let Some(exe) = paths.into_iter().find(|p| p.is_file()).or_else(|| crate::platform::find_on_path(cmd)) {
            push(name.to_string(), exe, &mut out);
        }
    }

    // Visual Studio: <Program Files>\Microsoft Visual Studio\<versão>\<edição>\Common7\IDE\devenv.exe
    for root in [pf.join("Microsoft Visual Studio"), pf86.join("Microsoft Visual Studio")] {
        for version in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            for edition in std::fs::read_dir(version.path()).into_iter().flatten().flatten() {
                let exe = edition.path().join(r"Common7\IDE\devenv.exe");
                if exe.is_file() {
                    let name = format!("Visual Studio {} {}", version.file_name().to_string_lossy(), edition.file_name().to_string_lossy());
                    push(name, exe, &mut out);
                }
            }
        }
    }

    // JetBrains (IntelliJ, WebStorm, PyCharm, Rider…): <dir>\bin\<ide>64.exe,
    // installed directly or by the Toolbox.
    let mut jet_dirs: Vec<PathBuf> = std::fs::read_dir(pf.join("JetBrains")).into_iter().flatten().flatten().map(|d| d.path()).collect();
    for app in std::fs::read_dir(local.join(r"Programs")).into_iter().flatten().flatten() {
        if app.path().join("bin").is_dir() && app.path().join("product-info.json").is_file() {
            jet_dirs.push(app.path());
        }
    }
    for dir in jet_dirs {
        let Some(exe) = std::fs::read_dir(dir.join("bin")).into_iter().flatten().flatten().map(|f| f.path()).find(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.ends_with("64.exe") && !n.starts_with("fsnotifier") && !n.starts_with("elevator") && !n.starts_with("restarter")
        }) else {
            continue;
        };
        push(dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "JetBrains".into()), exe, &mut out);
    }
    out
}

/// The editor projects open in: the one chosen in Ajustes (if it is still
/// installed), else the first one found. Returns the program and its name.
pub fn code_editor() -> Option<(std::path::PathBuf, String)> {
    let chosen = crate::settings::load().jarvis_code_editor;
    let all = installed_editors();
    let pick = all
        .iter()
        .find(|e| !chosen.is_empty() && e.path.eq_ignore_ascii_case(&chosen))
        .or_else(|| all.first())?;
    Some((std::path::PathBuf::from(&pick.path), pick.name.clone()))
}

/// Opens a project folder in the code editor. The path goes as its own
/// argument (no shell), so `&` or `%` in a folder name stay part of the name.
pub fn open_in_editor(path: &str) -> Result<String, String> {
    let dir = std::path::Path::new(path);
    if !(dir.is_absolute() && dir.exists()) {
        return Err(format!("Pasta não encontrada: {path}"));
    }
    let (exe, name) = code_editor().ok_or("Não encontrei nenhum editor de código instalado (Antigravity, VS Code, Cursor…).")?;
    let mut cmd = Command::new(&exe);
    cmd.arg(path);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map_err(|e| format!("Erro ao abrir o {name}: {e}"))?;
    Ok(format!("Abrindo no {name}: {path}"))
}

/// Opens a known Windows app, URL, or friendly alias (e.g. "spotify", "chrome", "youtube").
#[tauri::command]
pub fn jarvis_open_target(target: String) -> Result<String, String> {
    let raw = target.trim();
    if raw.is_empty() {
        return Err("Alvo vazio".into());
    }
    let lower = raw.to_lowercase();

    // Direct web URLs
    if lower.starts_with("http://") || lower.starts_with("https://") {
        launch_shell(raw)?;
        return Ok(format!("Abrindo {raw}"));
    }

    // Search & IDE deep links: youtube:..., google:..., spotify:..., vscode:...
    if let Some(q) = raw.strip_prefix("youtube:").or_else(|| raw.strip_prefix("YOUTUBE:")) {
        let encoded = q.trim().replace(' ', "+");
        let url = format!("https://www.youtube.com/results?search_query={encoded}");
        launch_shell(&url)?;
        return Ok(format!("Pesquisando no YouTube: {}", q.trim()));
    }
    if let Some(q) = raw.strip_prefix("google:").or_else(|| raw.strip_prefix("GOOGLE:")) {
        let encoded = q.trim().replace(' ', "+");
        let url = format!("https://www.google.com/search?q={encoded}");
        launch_shell(&url)?;
        return Ok(format!("Pesquisando no Google: {}", q.trim()));
    }
    if let Some(q) = raw.strip_prefix("spotify:").or_else(|| raw.strip_prefix("SPOTIFY:")) {
        let tail = q.trim();
        if tail.is_empty() {
            launch_shell("spotify:")?;
            return Ok("Abrindo Spotify".into());
        }
        let lower_tail = tail.to_lowercase();
        if lower_tail.starts_with("track:")
            || lower_tail.starts_with("album:")
            || lower_tail.starts_with("playlist:")
            || lower_tail.starts_with("artist:")
            || lower_tail.starts_with("episode:")
            || lower_tail.starts_with("show:")
            || lower_tail.starts_with("search:")
        {
            let uri = format!("spotify:{tail}");
            launch_shell(&uri)?;
            return Ok(format!("Abrindo no Spotify: {tail}"));
        }
        let encoded = tail.replace(' ', "%20");
        let uri = format!("spotify:search:{encoded}");
        launch_shell(&uri)?;
        return Ok(format!("Buscando no Spotify: {tail}"));
    }
    // A project in the code editor (Antigravity when installed, else VS Code).
    let editor_prefix = ["vscode:", "editor:", "antigravity:"].iter().find_map(|pre| {
        raw.get(..pre.len()).filter(|h| h.eq_ignore_ascii_case(pre)).map(|_| &raw[pre.len()..])
    });
    if let Some(p) = editor_prefix {
        return open_in_editor(p.trim());
    }

    // Friendly aliases -> executable / protocol / URL
    let resolved: (&str, &str) = match lower.as_str() {
        "spotify" => ("spotify:", "Spotify"),
        "chrome" | "google chrome" => ("chrome", "Google Chrome"),
        "edge" | "microsoft edge" => ("msedge", "Microsoft Edge"),
        "google" => ("https://www.google.com", "Google"),
        "youtube" => ("https://www.youtube.com", "YouTube"),
        "gmail" => ("https://mail.google.com", "Gmail"),
        "whatsapp" => ("https://web.whatsapp.com", "WhatsApp"),
        "discord" => ("discord:", "Discord"),
        "vscode" | "vs code" | "code" => ("code", "VS Code"),
        "notepad" | "bloco de notas" => ("notepad", "Bloco de Notas"),
        "calc" | "calculadora" => ("calc", "Calculadora"),
        "explorer" | "arquivos" | "explorador" => ("explorer", "Explorador de Arquivos"),
        "terminal" | "wt" => ("wt", "Terminal"),
        "powershell" => ("powershell", "PowerShell"),
        "cmd" => ("cmd", "Prompt de Comando"),
        _ => {
            // If it looks like a domain (e.g. "github.com"), open as https://
            if !lower.contains(' ') && lower.contains('.') && !std::path::Path::new(raw).exists() {
                let url = format!("https://{raw}");
                launch_shell(&url)?;
                return Ok(format!("Abrindo {url}"));
            }
            // An installed app, by the name in the Start menu / Desktop.
            if !std::path::Path::new(raw).exists() {
                if let Some(lnk) = find_app_shortcut(raw) {
                    let label = lnk.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| raw.to_string());
                    launch_shell(&lnk.to_string_lossy())?;
                    return Ok(format!("Abrindo {label}"));
                }
            }
            (raw, raw)
        }
    };

    if resolved.0 == "code" {
        #[cfg(windows)]
        {
            Command::new("cmd")
                .args(["/C", "code"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("Falha ao abrir VS Code: {e}"))?;
            return Ok(format!("Abrindo {}", resolved.1));
        }
    }
    launch_shell(resolved.0)?;
    Ok(format!("Abrindo {}", resolved.1))
}

fn launch_shell(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: isize,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        fn to_wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }
        let op_w = to_wide("open");
        let target_w = to_wide(target);
        let res = unsafe {
            ShellExecuteW(
                0,
                op_w.as_ptr(),
                target_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1, // SW_SHOWNORMAL
            )
        };
        if res <= 32 {
            return Err(format!("Falha ao abrir {target} (ShellExecuteW={res})"));
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(target)
            .spawn()
            .map_err(|e| format!("Falha ao abrir {target}: {e}"))?;
        Ok(())
    }
}

fn truncate_output(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(max_chars).collect();
        format!("{head}\n… [saída truncada]")
    }
}

/// Executes a user-confirmed PC action (`powershell`, `cmd`, `open`, `process`, `file`)
/// and returns captured output or status.
pub fn execute_pc_action(kind: &str, command: &str) -> Result<String, String> {
    let clean_kind = kind.trim().to_lowercase();
    let clean_cmd = command.trim();
    if clean_cmd.is_empty() {
        return Err("Comando vazio".into());
    }

    match clean_kind.as_str() {
        "open" | "app" | "url" => jarvis_open_target(clean_cmd.to_string()),
        "powershell" | "ps" | "terminal" => {
            #[cfg(windows)]
            {
                use std::io::Read;
                use std::process::Stdio;

                let full_cmd = format!(
                    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8; {clean_cmd}"
                );
                let mut child = Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &full_cmd])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| format!("Falha ao executar PowerShell: {e}"))?;

                let out_reader = child.stdout.take().map(|mut s| {
                    std::thread::spawn(move || {
                        let mut buf = Vec::new();
                        let _ = s.read_to_end(&mut buf);
                        buf
                    })
                });
                let err_reader = child.stderr.take().map(|mut s| {
                    std::thread::spawn(move || {
                        let mut buf = Vec::new();
                        let _ = s.read_to_end(&mut buf);
                        buf
                    })
                });

                let start = std::time::Instant::now();
                let mut timed_out = false;
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(st)) => break Some(st),
                        Ok(None) => {
                            if start.elapsed() >= Duration::from_secs(20) {
                                let _ = child.kill();
                                let _ = child.wait();
                                timed_out = true;
                                break None;
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(_) => break None,
                    }
                };

                let stdout_bytes = out_reader.and_then(|t| t.join().ok()).unwrap_or_default();
                let stderr_bytes = err_reader.and_then(|t| t.join().ok()).unwrap_or_default();
                let stdout = String::from_utf8_lossy(&stdout_bytes);
                let stderr = String::from_utf8_lossy(&stderr_bytes);

                if timed_out {
                    let partial = truncate_output(
                        if stderr.trim().is_empty() { &stdout } else { &stderr },
                        1000,
                    );
                    return Err(if partial.is_empty() {
                        "PowerShell excedeu o tempo limite de 20s e foi encerrado.".to_string()
                    } else {
                        format!("PowerShell excedeu 20s: {partial}")
                    });
                }

                let ok = status.is_some_and(|st| st.success());
                if ok {
                    let msg = truncate_output(&stdout, 3500);
                    Ok(if msg.is_empty() {
                        format!("Comando executado com sucesso: {clean_cmd}")
                    } else {
                        msg
                    })
                } else {
                    let err_msg = truncate_output(
                        if stderr.trim().is_empty() { &stdout } else { &stderr },
                        2000,
                    );
                    Err(if err_msg.is_empty() {
                        format!("PowerShell encerrou com código {:?}", status.and_then(|s| s.code()))
                    } else {
                        err_msg
                    })
                }
            }
            #[cfg(not(windows))]
            {
                Err("Automação PowerShell disponível apenas no Windows".into())
            }
        }
        "cmd" => {
            #[cfg(windows)]
            {
                let out = Command::new("cmd")
                    .args(["/C", clean_cmd])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .map_err(|e| format!("Falha ao executar CMD: {e}"))?;
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if out.status.success() {
                    let msg = truncate_output(&stdout, 3500);
                    Ok(if msg.is_empty() {
                        format!("Comando executado com sucesso: {clean_cmd}")
                    } else {
                        msg
                    })
                } else {
                    let err_msg = truncate_output(
                        if stderr.trim().is_empty() { &stdout } else { &stderr },
                        2000,
                    );
                    Err(if err_msg.is_empty() {
                        format!("CMD encerrou com código {:?}", out.status.code())
                    } else {
                        err_msg
                    })
                }
            }
            #[cfg(not(windows))]
            {
                let out = Command::new("sh")
                    .args(["-c", clean_cmd])
                    .output()
                    .map_err(|e| format!("Falha ao executar shell: {e}"))?;
                let stdout = String::from_utf8_lossy(&out.stdout);
                if out.status.success() {
                    Ok(truncate_output(&stdout, 3500))
                } else {
                    Err(truncate_output(&String::from_utf8_lossy(&out.stderr), 2000))
                }
            }
        }
        "process" => {
            #[cfg(windows)]
            {
                let ps_script = if let Some(target) = clean_cmd
                    .strip_prefix("kill:")
                    .or_else(|| clean_cmd.strip_prefix("KILL:"))
                {
                    let t = target.trim().replace('\'', "''");
                    if t.chars().all(|c| c.is_ascii_digit()) {
                        format!("Stop-Process -Id {t} -Force; 'Processo PID {t} encerrado.'")
                    } else {
                        format!("Stop-Process -Name '{t}' -Force; 'Processo {t} encerrado.'")
                    }
                } else {
                    clean_cmd.to_string()
                };
                execute_pc_action("powershell", &ps_script)
            }
            #[cfg(not(windows))]
            {
                Err("Gerenciamento de processos implementado para Windows".into())
            }
        }
        "file" => {
            if let Some(path) = clean_cmd.strip_prefix("open:") {
                launch_shell(path.trim())?;
                Ok(format!("Abrindo {}", path.trim()))
            } else if let Some(path) = clean_cmd.strip_prefix("mkdir:") {
                std::fs::create_dir_all(path.trim())
                    .map_err(|e| format!("Erro ao criar pasta: {e}"))?;
                Ok(format!("Pasta criada: {}", path.trim()))
            } else if let Some(path) = clean_cmd.strip_prefix("delete:") {
                let p = std::path::Path::new(path.trim());
                if p.is_dir() {
                    std::fs::remove_dir_all(p)
                        .map_err(|e| format!("Erro ao remover pasta: {e}"))?;
                } else {
                    std::fs::remove_file(p)
                        .map_err(|e| format!("Erro ao remover arquivo: {e}"))?;
                }
                Ok(format!("Removido: {}", path.trim()))
            } else if let Some(rest) = clean_cmd.strip_prefix("move:") {
                let (src, dst) = rest
                    .split_once("->")
                    .or_else(|| rest.split_once('|'))
                    .ok_or_else(|| "Formato esperado: move:origem->destino".to_string())?;
                std::fs::rename(src.trim(), dst.trim())
                    .map_err(|e| format!("Erro ao mover: {e}"))?;
                Ok(format!("Movido de {} para {}", src.trim(), dst.trim()))
            } else {
                execute_pc_action("powershell", clean_cmd)
            }
        }
        other => Err(format!("Tipo de ação desconhecido: {other}")),
    }
}

/// Sets the Windows master audio volume (0..=100) instantly via CoreAudio `IAudioEndpointVolume`
/// on a dedicated STA thread so it never conflicts with Tokio/WinRT COM apartments.
#[tauri::command]
pub fn jarvis_set_volume(percent: u32) -> Result<String, String> {
    let clamped = percent.min(100);
    #[cfg(windows)]
    {
        let res = std::thread::spawn(move || -> Result<(), String> {
            use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
            use windows::Win32::Media::Audio::{
                eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
            };
            use windows::Win32::System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            };

            unsafe {
                let hr_init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let should_uninit = hr_init.is_ok();
                let inner: Result<(), String> = (|| {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                            .map_err(|e| format!("MMDeviceEnumerator: {e}"))?;
                    let device = enumerator
                        .GetDefaultAudioEndpoint(eRender, eMultimedia)
                        .map_err(|e| format!("GetDefaultAudioEndpoint: {e}"))?;
                    let endpoint: IAudioEndpointVolume = device
                        .Activate(CLSCTX_ALL, None)
                        .map_err(|e| format!("Activate IAudioEndpointVolume: {e}"))?;
                    endpoint
                        .SetMasterVolumeLevelScalar(clamped as f32 / 100.0, std::ptr::null())
                        .map_err(|e| format!("SetMasterVolumeLevelScalar: {e}"))?;
                    if clamped > 0 {
                        let _ = endpoint.SetMute(false, std::ptr::null());
                    }
                    Ok(())
                })();
                if should_uninit {
                    CoUninitialize();
                }
                inner
            }
        })
        .join()
        .map_err(|_| "Falha na thread de áudio".to_string())?;
        res?;
        return Ok(format!("Volume ajustado para {clamped}%"));
    }
    #[cfg(not(windows))]
    {
        Err("Controle de volume nativo implementado para Windows".into())
    }
}

/// Reads the current Windows master audio volume (0..=100) via CoreAudio on a clean STA thread.
pub fn get_system_volume() -> Option<u32> {
    #[cfg(windows)]
    {
        std::thread::spawn(|| -> Option<u32> {
            use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
            use windows::Win32::Media::Audio::{
                eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
            };
            use windows::Win32::System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            };

            unsafe {
                let hr_init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let should_uninit = hr_init.is_ok();
                let vol = (|| -> Option<u32> {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                    let device = enumerator
                        .GetDefaultAudioEndpoint(eRender, eMultimedia)
                        .ok()?;
                    let endpoint: IAudioEndpointVolume =
                        device.Activate(CLSCTX_ALL, None).ok()?;
                    let scalar = endpoint.GetMasterVolumeLevelScalar().ok()?;
                    Some((scalar * 100.0).round().clamp(0.0, 100.0) as u32)
                })();
                if should_uninit {
                    CoUninitialize();
                }
                vol
            }
        })
        .join()
        .ok()
        .flatten()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Lists up to 15 visible top-level window titles + the foreground window title in <5ms via Win32.
pub fn list_visible_windows() -> Vec<String> {
    #[cfg(windows)]
    {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetForegroundWindow, GetWindowTextW, IsWindowVisible,
        };

        struct CollectState {
            fg: isize,
            titles: Vec<String>,
        }

        unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let state = &mut *(lparam.0 as *mut CollectState);
            if state.titles.len() >= 15 {
                return BOOL(0);
            }
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut buf = [0u16; 256];
            let len = GetWindowTextW(hwnd, &mut buf);
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]).trim().to_string();
                if !title.is_empty() && !state.titles.iter().any(|t| t == &title) {
                    if hwnd.0 as isize == state.fg {
                        state.titles.push(format!("[FOCO] {title}"));
                    } else {
                        state.titles.push(title);
                    }
                }
            }
            BOOL(1)
        }

        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        let mut state = CollectState {
            fg,
            titles: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&mut state as *mut CollectState as isize));
        }
        state.titles
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Builds a concise live snapshot of the user's PC state (audio, Spotify, open windows, folders,
/// and optionally top processes / clipboard) for the agentic brain.
pub fn get_live_pc_snapshot(include_processes: bool, include_clipboard: bool) -> String {
    let mut lines = Vec::new();

    if let Some(vol) = get_system_volume() {
        lines.push(format!("- Volume mestre do Windows: {vol}%"));
    }

    if let Some(track) = crate::spotify::current_cached_track_summary() {
        lines.push(format!("- Spotify atual: {track}"));
    } else {
        lines.push("- Spotify atual: nada reproduzindo no cache recente".to_string());
    }

    let wins = list_visible_windows();
    if !wins.is_empty() {
        lines.push(format!("- Janelas visíveis: {}", wins.join(" | ")));
    }

    if let Ok(profile) = std::env::var("USERPROFILE") {
        // The projects folder set in Ajustes (D:\Projetos when it exists, else ~\Projetos).
        let projects = crate::jarvis_config::projects_dir();
        lines.push(format!(
            r"- Pastas do usuário: USERPROFILE={profile}, Desktop={profile}\Desktop, Downloads={profile}\Downloads, Documents={profile}\Documents, Projetos={}",
            projects.display()
        ));
    }

    #[cfg(windows)]
    if include_processes {
        let cmd = r#"Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 10 Name, Id, @{N='MB';E={[math]::Round($_.WorkingSet64/1MB,1)}} | ForEach-Object { "$($_.Name) (PID $($_.Id)): $($_.MB) MB" }"#;
        if let Ok(procs) = execute_pc_action("powershell", cmd) {
            lines.push(format!("- Top 10 processos por memória:\n{}", procs.trim()));
        }
    }

    #[cfg(windows)]
    if include_clipboard {
        if let Ok(clip) = execute_pc_action("powershell", "Get-Clipboard -Raw") {
            let trimmed = clip.trim();
            if !trimmed.is_empty() {
                lines.push(format!("- Área de transferência:\n{}", truncate_output(trimmed, 800)));
            } else {
                lines.push("- Área de transferência: vazia".to_string());
            }
        }
    }

    lines.join("
")
}

/// Controls system media playback and volume via Windows virtual media keys.
#[tauri::command]
pub fn jarvis_media_control(action: String) -> Result<String, String> {
    let lower = action.trim().to_lowercase();
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
            VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_VOLUME_DOWN,
            VK_VOLUME_MUTE, VK_VOLUME_UP,
        };

        let (vk, label): (VIRTUAL_KEY, &str) = match lower.as_str() {
            "play" | "pause" | "play_pause" | "toggle" => (VK_MEDIA_PLAY_PAUSE, "Play/Pause"),
            "next" | "proxima" | "próxima" => (VK_MEDIA_NEXT_TRACK, "Próxima faixa"),
            "prev" | "previous" | "anterior" => (VK_MEDIA_PREV_TRACK, "Faixa anterior"),
            "vol_up" | "volume_up" | "aumentar" => (VK_VOLUME_UP, "Aumentar volume"),
            "vol_down" | "volume_down" | "diminuir" => (VK_VOLUME_DOWN, "Diminuir volume"),
            "mute" | "mudo" => (VK_VOLUME_MUTE, "Mudo"),
            _ => return Err(format!("Ação de mídia desconhecida: {action}")),
        };

        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: Default::default(),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];

        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
        return Ok(label.to_string());
    }

    #[cfg(not(windows))]
    {
        Err("Controle de mídia nativo implementado para Windows".into())
    }
}

/// Captures the primary screen as a base64-encoded PNG for Gemini Vision ("Ler tela").
#[tauri::command]
pub fn jarvis_capture_screen() -> Result<String, String> {
    #[cfg(windows)]
    {
        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$ms = New-Object System.IO.MemoryStream
$bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
[Convert]::ToBase64String($ms.ToArray())
"#;
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Falha ao capturar tela: {e}"))?;
        if !out.status.success() {
            return Err("Falha ao capturar tela".into());
        }
        let b64 = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if b64.is_empty() {
            return Err("Captura de tela vazia".into());
        }
        return Ok(b64);
    }
    #[cfg(not(windows))]
    {
        Err("Captura de tela implementada para Windows".into())
    }
}

/// Reads the current plain-text clipboard content on Windows.
#[tauri::command]
pub fn jarvis_get_clipboard() -> Result<String, String> {
    #[cfg(windows)]
    {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard -Raw"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Falha ao ler área de transferência: {e}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcStats {
    pub cpu_percent: u32,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub mem_percent: u32,
    pub disk_free_gb: f64,
    pub disk_total_gb: f64,
    pub disk_percent: u32,
}

/// Returns CPU, RAM and C: disk usage for the "Meu PC" dashboard tab.
#[tauri::command]
pub async fn jarvis_pc_stats() -> Result<PcStats, String> {
    #[cfg(windows)]
    {
        return tokio::task::spawn_blocking(|| -> Result<PcStats, String> {
            let script = r#"
$os = Get-CimInstance Win32_OperatingSystem
$cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
$disk = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='C:'"
$memTotal = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
$memFree = [math]::Round($os.FreePhysicalMemory / 1MB, 1)
$memUsed = [math]::Round($memTotal - $memFree, 1)
$memPct = if ($memTotal -gt 0) { [math]::Round(($memUsed / $memTotal) * 100) } else { 0 }
$diskTotal = [math]::Round($disk.Size / 1GB, 0)
$diskFree = [math]::Round($disk.FreeSpace / 1GB, 0)
$diskPct = if ($diskTotal -gt 0) { [math]::Round((($diskTotal - $diskFree) / $diskTotal) * 100) } else { 0 }
@{
  cpuPercent = [int]$cpu
  memUsedGb = $memUsed
  memTotalGb = $memTotal
  memPercent = [int]$memPct
  diskFreeGb = $diskFree
  diskTotalGb = $diskTotal
  diskPercent = [int]$diskPct
} | ConvertTo-Json -Compress
"#;
            let out = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("Falha ao ler PC stats: {e}"))?;
            let raw = String::from_utf8_lossy(&out.stdout);
            let v: serde_json::Value =
                serde_json::from_str(raw.trim()).map_err(|e| format!("JSON stats: {e}"))?;
            Ok(PcStats {
                cpu_percent: v["cpuPercent"].as_u64().unwrap_or(0) as u32,
                mem_used_gb: v["memUsedGb"].as_f64().unwrap_or(0.0),
                mem_total_gb: v["memTotalGb"].as_f64().unwrap_or(0.0),
                mem_percent: v["memPercent"].as_u64().unwrap_or(0) as u32,
                disk_free_gb: v["diskFreeGb"].as_f64().unwrap_or(0.0),
                disk_total_gb: v["diskTotalGb"].as_f64().unwrap_or(0.0),
                disk_percent: v["diskPercent"].as_u64().unwrap_or(0) as u32,
            })
        })
        .await
        .map_err(|e| format!("join stats: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        Ok(PcStats {
            cpu_percent: 0,
            mem_used_gb: 0.0,
            mem_total_gb: 0.0,
            mem_percent: 0,
            disk_free_gb: 0.0,
            disk_total_gb: 0.0,
            disk_percent: 0,
        })
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub cover_url: String,
}

static COVER_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Reads the live Windows Media Session (Spotify / Browser / Media Player) via GSMTC
/// and resolves the album cover URL (cached per track).
#[tauri::command]
pub async fn jarvis_now_playing() -> Result<NowPlaying, String> {
    #[cfg(windows)]
    {
        let raw_info = tokio::task::spawn_blocking(|| -> Option<(String, String, String, bool)> {
            let script = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($WinRtTask, $ResultType) {
    $asTask = $asTaskGeneric.MakeGenericMethod($ResultType)
    $netTask = $asTask.Invoke($null, @($WinRtTask))
    $netTask.Wait(-1) | Out-Null
    $netTask.Result
}
[Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,Windows.Media.Control,ContentType=WindowsRuntime] | Out-Null
$mgr = Await ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
$session = $mgr.GetCurrentSession()
if ($session) {
    $props = Await ($session.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
    $info = $session.GetPlaybackInfo()
    @{ title = [string]$props.Title; artist = [string]$props.Artist; album = [string]$props.AlbumTitle; playing = ($info.PlaybackStatus.ToString() -eq 'Playing') } | ConvertTo-Json -Compress
}
"#;
            let out = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let v: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
            let title = v["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() {
                return None;
            }
            let artist = v["artist"].as_str().unwrap_or("").trim().to_string();
            let album = v["album"].as_str().unwrap_or("").trim().to_string();
            let playing = v["playing"].as_bool().unwrap_or(false);
            Some((title, artist, album, playing))
        })
        .await
        .map_err(|e| e.to_string())?;

        let Some((title, artist, album, playing)) = raw_info else {
            return Ok(NowPlaying::default());
        };

        let cache_key = format!("{artist} - {title}");
        if let Some(cached) = COVER_CACHE.lock().unwrap().get(&cache_key).cloned() {
            return Ok(NowPlaying {
                title,
                artist,
                album,
                playing,
                cover_url: cached,
            });
        }

        // Resolve cover art once per track via iTunes public search API
        let mut cover_url = String::new();
        if let Ok(client) = reqwest::Client::builder()
            .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .timeout(Duration::from_secs(4))
            .build()
        {
            if let Ok(mut url) = reqwest::Url::parse("https://itunes.apple.com/search") {
                url.query_pairs_mut()
                    .append_pair("term", &cache_key)
                    .append_pair("entity", "song")
                    .append_pair("limit", "1");
                if let Ok(res) = client.get(url).send().await {
                    if let Ok(json) = res.json::<serde_json::Value>().await {
                        if let Some(art) = json["results"][0]["artworkUrl100"].as_str() {
                            cover_url = art.replace("100x100bb", "300x300bb");
                        }
                    }
                }
            }
        }
        COVER_CACHE.lock().unwrap().insert(cache_key, cover_url.clone());

        return Ok(NowPlaying {
            title,
            artist,
            album,
            playing,
            cover_url,
        });
    }
    #[cfg(not(windows))]
    {
        Ok(NowPlaying::default())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// `cargo test finds_this_pc_apps -- --ignored --nocapture` (looks, opens nothing).
    #[test]
    #[ignore]
    fn finds_this_pc_apps() {
        for name in ["OpenBot", "Obsidian", "Steam", "Discord", "visual studio code", "nada que exista aqui"] {
            println!("{name:>22} → {:?}", find_app_shortcut(name));
        }
    }

    #[test]
    fn apps_are_found_by_their_shortcut_name() {
        let list: Vec<(String, PathBuf)> = ["openbot", "obsidian", "visual studio code", "fivem - cfx re development kit (fxdk)", "fivem", "steam"]
            .iter()
            .map(|n| (n.to_string(), PathBuf::from(format!("{n}.lnk"))))
            .collect();
        let pick = |q: &str| pick_shortcut(&list, &app_query(q)).map(|p| p.to_string_lossy().to_string());
        assert_eq!(pick("OpenBot").as_deref(), Some("openbot.lnk"));
        assert_eq!(pick("o open bot").as_deref(), Some("openbot.lnk"));
        assert_eq!(pick("FiveM").as_deref(), Some("fivem.lnk"));
        assert_eq!(pick("visual studio").as_deref(), Some("visual studio code.lnk"));
        assert_eq!(pick("xyz"), None);
    }
}

#[cfg(test)]
mod editor_probe {
    /// What this PC has (`cargo test editors_on_this_pc -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn editors_on_this_pc() {
        for e in super::installed_editors() {
            println!("{} -> {}", e.name, e.path);
        }
    }
}
