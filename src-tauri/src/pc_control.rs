//! Jarvis using the PC's programs the way a person does: find a program's
//! window, read what it shows (Windows UI Automation), click its buttons by
//! name and type into it. Plus reading and editing text files, with a backup
//! of every file before it changes.
//!
//! Example: "calcula 125 vezes 8 na calculadora" → open_target "calc",
//! app_type("calculadora", "125*8="), app_read("calculadora") → "1.000".

use crate::file_search::fold;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- windows

/// Names people say → words in the window title or the program's file name.
const ALIASES: &[&[&str]] = &[
    &["calculadora", "calculator", "calc"],
    &["bloco de notas", "notepad", "bloco"],
    &["explorador de arquivos", "explorer", "explorador", "arquivos"],
    &["word", "winword"],
    &["excel"],
    &["powerpoint", "powerpnt"],
    &["paint", "mspaint"],
    &["configuracoes", "settings", "systemsettings"],
    &["navegador", "chrome", "msedge", "edge", "firefox"],
    &["terminal", "windowsterminal", "powershell", "cmd"],
    &["vs code", "vscode", "code", "visual studio code"],
];

/// What to look for in titles / file names for what the user said.
fn wanted(query: &str) -> Vec<String> {
    let q = fold(query.trim());
    let mut out = vec![q.clone()];
    for group in ALIASES {
        if group.iter().any(|a| *a == q) {
            out.extend(group.iter().map(|a| a.to_string()));
        }
    }
    out.retain(|s| !s.is_empty());
    out.dedup();
    out
}

#[derive(Clone, Debug)]
pub struct Win {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
}

/// Whether a window (title, exe stem) is the one asked for. 2 = by title, 1 = by exe.
fn matches(title: &str, exe: &str, want: &[String]) -> u8 {
    let t = fold(title);
    let e = fold(exe);
    if want.iter().any(|w| t.contains(w.as_str())) {
        2
    } else if want.iter().any(|w| e == *w) {
        1
    } else {
        0
    }
}

#[cfg(windows)]
mod win {
    use super::Win;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    fn exe_of(pid: u32) -> String {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(h);
            if !ok {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        }
    }

    /// Top-level windows a person can see, top of the z-order first (not Jarvis's own).
    pub fn windows() -> Vec<Win> {
        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let out = &mut *(lparam.0 as *mut Vec<(isize, String, u32)>);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
            if cloaked != 0 {
                return BOOL(1);
            }
            let mut buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, &mut buf);
            if len <= 0 {
                return BOOL(1);
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL(1);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            out.push((hwnd.0 as isize, String::from_utf16_lossy(&buf[..len as usize]).trim().to_string(), pid));
            BOOL(1)
        }
        let mut raw: Vec<(isize, String, u32)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut raw as *mut _ as isize));
        }
        let me = unsafe { GetCurrentProcessId() };
        raw.into_iter()
            .filter(|(_, t, pid)| *pid != me && !t.is_empty() && t != "Program Manager")
            .map(|(hwnd, title, pid)| Win { hwnd, title, exe: exe_of(pid) })
            .collect()
    }

    pub fn foreground() -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        unsafe {
            SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// Brings the window to the front (Windows only allows it in some cases:
    /// the input-thread trick and, last, a tap of Alt).
    pub fn focus(hwnd: isize) -> bool {
        let h = HWND(hwnd as *mut _);
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            if GetForegroundWindow() == h {
                return true;
            }
            let _ = SetForegroundWindow(h);
            std::thread::sleep(std::time::Duration::from_millis(120));
            if GetForegroundWindow() == h {
                return true;
            }
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            if fg_thread != 0 && fg_thread != me {
                let _ = AttachThreadInput(me, fg_thread, true);
                let _ = BringWindowToTop(h);
                let _ = SetForegroundWindow(h);
                let _ = AttachThreadInput(me, fg_thread, false);
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
            if GetForegroundWindow() == h {
                return true;
            }
            send(&[key(VK_MENU, false), key(VK_MENU, true)]);
            let _ = SetForegroundWindow(h);
            std::thread::sleep(std::time::Duration::from_millis(150));
            GetForegroundWindow() == h
        }
    }

    /// Types text as characters (any language, symbols included).
    pub fn type_text(text: &str) {
        for c in text.chars() {
            match c {
                '\n' => send(&[key(VK_RETURN, false), key(VK_RETURN, true)]),
                '\t' => send(&[key(VK_TAB, false), key(VK_TAB, true)]),
                '\r' => {}
                _ => {
                    let mut units = [0u16; 2];
                    for u in c.encode_utf16(&mut units).iter() {
                        let mk = |up: bool| INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: VIRTUAL_KEY(0),
                                    wScan: *u,
                                    dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE },
                                    time: 0,
                                    dwExtraInfo: 0,
                                },
                            },
                        };
                        send(&[mk(false), mk(true)]);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
    }

    fn vk_of(name: &str) -> Option<VIRTUAL_KEY> {
        let n = name.trim().to_lowercase();
        Some(match n.as_str() {
            "enter" | "return" | "entrar" => VK_RETURN,
            "tab" => VK_TAB,
            "esc" | "escape" => VK_ESCAPE,
            "backspace" | "apagar" => VK_BACK,
            "delete" | "del" => VK_DELETE,
            "space" | "espaco" | "espaço" => VK_SPACE,
            "up" | "cima" => VK_UP,
            "down" | "baixo" => VK_DOWN,
            "left" | "esquerda" => VK_LEFT,
            "right" | "direita" => VK_RIGHT,
            "home" => VK_HOME,
            "end" => VK_END,
            "pageup" | "pgup" => VK_PRIOR,
            "pagedown" | "pgdn" => VK_NEXT,
            "insert" => VK_INSERT,
            "ctrl" | "control" | "controle" => VK_CONTROL,
            "shift" => VK_SHIFT,
            "alt" => VK_MENU,
            "win" | "windows" => VK_LWIN,
            "printscreen" => VK_SNAPSHOT,
            _ if n.len() >= 2 && n.starts_with('f') && n[1..].parse::<u16>().is_ok_and(|k| (1..=24).contains(&k)) => {
                VIRTUAL_KEY(VK_F1.0 + n[1..].parse::<u16>().unwrap() - 1)
            }
            _ if n.chars().count() == 1 => {
                let c = n.chars().next()?.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    VIRTUAL_KEY(c as u16)
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }

    /// "ctrl+s", "alt+f4", "ctrl+a delete", "enter": combos separated by spaces.
    pub fn press(keys: &str) -> Result<(), String> {
        let mut combos = Vec::new();
        for combo in keys.split_whitespace() {
            let parts: Vec<VIRTUAL_KEY> = combo
                .split('+')
                .filter(|p| !p.is_empty())
                .map(|p| vk_of(p).ok_or_else(|| format!("Não conheço a tecla \"{p}\".")))
                .collect::<Result<_, _>>()?;
            combos.push(parts);
        }
        for parts in combos {
            let mut inputs: Vec<INPUT> = parts.iter().map(|k| key(*k, false)).collect();
            inputs.extend(parts.iter().rev().map(|k| key(*k, true)));
            send(&inputs);
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        Ok(())
    }

    /// A left click at a point of the screen.
    pub fn click_at(x: i32, y: i32) {
        unsafe {
            let _ = SetCursorPos(x, y);
        }
        let m = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        send(&[m(MOUSEEVENTF_LEFTDOWN), m(MOUSEEVENTF_LEFTUP)]);
    }
}

/// The window for what the user said ("calculadora", "bloco de notas", part
/// of a title); empty = the one in front. Waits a little for one just opened.
pub fn find_window(query: &str) -> Result<Win, String> {
    #[cfg(windows)]
    {
        let q = query.trim();
        for attempt in 0..20 {
            let list = win::windows();
            if q.is_empty() {
                let fg = win::foreground();
                if let Some(w) = list.iter().find(|w| w.hwnd == fg).or(list.first()) {
                    return Ok(w.clone());
                }
            } else {
                let want = wanted(q);
                let fg = win::foreground();
                let best = list
                    .iter()
                    .filter_map(|w| {
                        let m = matches(&w.title, &w.exe, &want);
                        (m > 0).then_some((m + u8::from(w.hwnd == fg), w))
                    })
                    .max_by_key(|(score, _)| *score);
                if let Some((_, w)) = best {
                    return Ok(w.clone());
                }
            }
            if attempt < 19 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        let open: Vec<String> = win::windows().into_iter().take(12).map(|w| w.title).collect();
        Err(format!("Não achei a janela \"{q}\". Janelas abertas: {}. Se o programa não está aberto, abra com open_target.", open.join(" | ")))
    }
    #[cfg(not(windows))]
    {
        let _ = query;
        Err("Disponível só no Windows.".into())
    }
}

// ------------------------------------------------------ UI Automation

#[derive(Clone, Debug)]
struct Element {
    name: String,
    kind: i32,
    value: String,
    rect: (i32, i32, i32, i32),
    enabled: bool,
}

fn kind_label(kind: i32) -> &'static str {
    match kind {
        50000 => "botão",
        50002 => "caixa",
        50003 => "lista suspensa",
        50004 => "campo",
        50005 => "link",
        50007 => "item",
        50011 => "menu",
        50013 => "opção",
        50019 => "aba",
        50020 => "texto",
        50024 => "item",
        50030 => "documento",
        50034 => "título",
        _ => "",
    }
}

fn clickable(kind: i32) -> bool {
    matches!(kind, 50000 | 50002 | 50003 | 50005 | 50007 | 50011 | 50013 | 50019 | 50024)
}

#[cfg(windows)]
mod uia {
    use super::Element;
    use windows::core::BSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::*;

    pub struct Tree {
        pub uia: IUIAutomation,
        pub nodes: Vec<(IUIAutomationElement, Element)>,
    }

    fn text_of(el: &IUIAutomationElement, kind: i32) -> String {
        unsafe {
            if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                if let Ok(s) = v.CurrentValue() {
                    let s = s.to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            if kind == 50030 || kind == 50004 {
                if let Ok(t) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                    if let Ok(r) = t.DocumentRange() {
                        if let Ok(s) = r.GetText(20_000) {
                            return s.to_string();
                        }
                    }
                }
            }
        }
        String::new()
    }

    /// The window's controls, in reading order (at most `max`).
    pub fn walk(hwnd: isize, max: usize) -> Result<Tree, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("UI Automation: {e}"))?;
            let root = uia.ElementFromHandle(HWND(hwnd as *mut _)).map_err(|e| format!("Janela: {e}"))?;
            let walker = uia.ControlViewWalker().map_err(|e| e.to_string())?;
            let mut nodes = Vec::new();
            // Depth-first, so the order is the order on screen.
            let mut stack: Vec<(IUIAutomationElement, usize)> = Vec::new();
            if let Ok(first) = walker.GetFirstChildElement(&root) {
                stack.push((first, 0));
            }
            let mut seen = 0usize;
            while let Some((el, depth)) = stack.pop() {
                seen += 1;
                if seen > max * 4 || nodes.len() >= max {
                    break;
                }
                if let Ok(next) = walker.GetNextSiblingElement(&el) {
                    stack.push((next, depth));
                }
                let offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
                let kind = el.CurrentControlType().map(|k| k.0).unwrap_or(0);
                if !offscreen {
                    let name = el.CurrentName().map(|b: BSTR| b.to_string()).unwrap_or_default();
                    let value = if matches!(kind, 50004 | 50030 | 50003) { text_of(&el, kind) } else { String::new() };
                    let r = el.CurrentBoundingRectangle().unwrap_or_default();
                    let enabled = el.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(true);
                    if !name.trim().is_empty() || !value.is_empty() || matches!(kind, 50004 | 50030) {
                        nodes.push((el.clone(), Element { name: name.trim().to_string(), kind, value, rect: (r.left, r.top, r.right, r.bottom), enabled }));
                    }
                }
                if depth < 30 && !offscreen {
                    if let Ok(child) = walker.GetFirstChildElement(&el) {
                        stack.push((child, depth + 1));
                    }
                }
            }
            Ok(Tree { uia, nodes })
        }
    }

    /// Presses a control: its own action first (no mouse), the mouse last.
    pub fn press(el: &IUIAutomationElement, e: &Element) -> bool {
        unsafe {
            if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
                if p.Invoke().is_ok() {
                    return true;
                }
            }
            if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
                if p.Toggle().is_ok() {
                    return true;
                }
            }
            if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
                if p.Select().is_ok() {
                    return true;
                }
            }
            if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
                if p.Expand().is_ok() {
                    return true;
                }
            }
        }
        let (l, t, r, b) = e.rect;
        if r > l && b > t {
            super::win::click_at((l + r) / 2, (t + b) / 2);
            return true;
        }
        false
    }

    pub fn set_focus(el: &IUIAutomationElement) {
        unsafe {
            let _ = el.SetFocus();
        }
    }
}

/// What a program's window shows: texts, buttons, fields (with what is in
/// them) — the result of the calculator, the text of a document…
pub fn read(app: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let w = find_window(app)?;
        let tree = uia::walk(w.hwnd, 400)?;
        let mut out = format!("Janela: \"{}\" ({})\n", w.title, w.exe);
        let mut last = String::new();
        for (_, e) in &tree.nodes {
            let label = kind_label(e.kind);
            let mut line = match (label.is_empty(), e.value.is_empty()) {
                (true, true) => e.name.clone(),
                (true, false) => format!("{}: {}", e.name, e.value),
                (false, true) => format!("[{label}] {}", e.name),
                (false, false) => format!("[{label}] {}: {}", e.name, e.value),
            };
            if !e.enabled {
                line.push_str(" (desativado)");
            }
            if line.trim().is_empty() || line == last {
                continue;
            }
            last = line.clone();
            out.push_str(&line.chars().take(4000).collect::<String>());
            out.push('\n');
            if out.len() > 12_000 {
                out.push_str("… (mais itens na janela)\n");
                break;
            }
        }
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Disponível só no Windows.".into())
    }
}

/// Score of a control's name against what was asked (0 = no).
fn name_score(name: &str, want: &str) -> u8 {
    let n = fold(name);
    let w = fold(want);
    if n.is_empty() || w.is_empty() {
        0
    } else if n == w {
        4
    } else if n.starts_with(&w) {
        3
    } else if n.split(|c: char| !c.is_alphanumeric()).any(|p| p == w) {
        2
    } else if n.contains(&w) {
        1
    } else {
        0
    }
}

/// Clicks a button / menu / tab / item of a program by its name.
pub fn click(app: &str, target: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let w = find_window(app)?;
        let tree = uia::walk(w.hwnd, 600)?;
        let best = tree
            .nodes
            .iter()
            .filter(|(_, e)| e.enabled)
            .filter_map(|(el, e)| {
                let s = name_score(&e.name, target);
                (s > 0).then_some((s * 2 + u8::from(clickable(e.kind)), el, e))
            })
            .max_by_key(|(s, _, _)| *s);
        let Some((_, el, e)) = best else {
            let names: Vec<&str> = tree.nodes.iter().filter(|(_, e)| clickable(e.kind) && !e.name.is_empty()).map(|(_, e)| e.name.as_str()).take(40).collect();
            return Err(format!("Não achei \"{target}\" em \"{}\". Opções: {}", w.title, names.join(", ")));
        };
        let _ = win::focus(w.hwnd);
        if !uia::press(el, e) {
            return Err(format!("Não consegui clicar em \"{}\".", e.name));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = &tree.uia;
        Ok(format!("Cliquei em \"{}\" ({}).", e.name, w.title))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, target);
        Err("Disponível só no Windows.".into())
    }
}

/// Types text and/or presses keys in a program (optionally in a named field).
pub fn type_into(app: &str, text: &str, keys: &str, field: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        if text.is_empty() && keys.trim().is_empty() {
            return Err("Nada para digitar.".into());
        }
        let w = find_window(app)?;
        if !win::focus(w.hwnd) {
            return Err(format!("O Windows não deixou trazer \"{}\" para a frente. Clique nela e peça de novo.", w.title));
        }
        if !field.trim().is_empty() {
            let tree = uia::walk(w.hwnd, 600)?;
            let hit = tree
                .nodes
                .iter()
                .filter(|(_, e)| matches!(e.kind, 50004 | 50030 | 50003))
                .filter_map(|(el, e)| {
                    let s = name_score(&e.name, field);
                    (s > 0).then_some((s, el, e))
                })
                .max_by_key(|(s, _, _)| *s);
            match hit {
                Some((_, el, e)) => {
                    uia::set_focus(el);
                    let (l, t, r, b) = e.rect;
                    if r > l && b > t {
                        win::click_at((l + r) / 2, (t + b) / 2);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                None => return Err(format!("Não achei o campo \"{field}\" em \"{}\".", w.title)),
            }
        }
        if !text.is_empty() {
            win::type_text(text);
        }
        if !keys.trim().is_empty() {
            win::press(keys)?;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let what = match (text.is_empty(), keys.trim().is_empty()) {
            (false, false) => format!("Digitei \"{text}\" e apertei {keys}"),
            (false, true) => format!("Digitei \"{text}\""),
            _ => format!("Apertei {keys}"),
        };
        Ok(format!("{what} em \"{}\". Use app_read para conferir o resultado.", w.title))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, text, keys, field);
        Err("Disponível só no Windows.".into())
    }
}

/// Brings forward a window of the program with this file name ("studio64").
/// False when it has no window yet (still starting).
pub fn focus_program(exe_stem: &str) -> bool {
    #[cfg(windows)]
    {
        let want = fold(exe_stem);
        if let Some(w) = win::windows().into_iter().find(|w| fold(&w.exe) == want) {
            let _ = win::focus(w.hwnd);
            return true;
        }
    }
    let _ = exe_stem;
    false
}

/// Types into whatever window is in front (routines, dictation).
pub fn type_here(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::type_text(text);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("Disponível só no Windows.".into())
    }
}

/// Presses keys / shortcuts in whatever window is in front.
pub fn press_here(keys: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::press(keys)
    }
    #[cfg(not(windows))]
    {
        let _ = keys;
        Err("Disponível só no Windows.".into())
    }
}

/// "Escreve: bom dia a todos", "digita aí 42": the words to type where the
/// cursor is. None = not dictation (writing an e-mail, a note, a message is
/// a job for the AI and its tools).
pub fn dictation(said: &str) -> Option<String> {
    let s = said.trim();
    let f = fold(s);
    let verbs = ["escreve", "escreva", "escrever", "digita", "digite", "digitar", "ditado"];
    let mut start = None;
    // Optional "Jarvis," in front.
    let lead = if f.starts_with("jarvis") { f.find(|c: char| c == ' ').map(|i| i + 1).unwrap_or(f.len()) } else { 0 };
    for v in verbs {
        if f[lead..].starts_with(v) && f[lead + v.len()..].starts_with([' ', ':', ',']) {
            start = Some(lead + v.len());
            break;
        }
    }
    let start = start?;
    // Same length in chars for the folded and the said text (fold maps 1:1).
    let mut rest: String = s.chars().skip(f[..start].chars().count()).collect();
    let mut rf = fold(&rest);
    for filler in [":", ",", " ai", " aí", " isso", " assim", " o seguinte", " pra mim", " para mim", ":", ","] {
        let ff = fold(filler);
        let t = rf.trim_start();
        if t.starts_with(ff.trim_start()) {
            let cut = rf.len() - t.len() + ff.trim_start().len();
            rest = rest.chars().skip(rf[..cut].chars().count()).collect();
            rf = fold(&rest);
        }
    }
    let text = rest.trim().trim_start_matches([':', ',']).trim().to_string();
    if text.is_empty() {
        return None;
    }
    let t = fold(&text);
    // "Escreve um e-mail para…", "escreve no WhatsApp…", "escreve uma nota…": not dictation.
    const NOT: &[&str] = &[
        "um email", "um e-mail", "uma mensagem", "um texto", "um resumo", "uma nota", "uma anotacao", "um lembrete", "um post", "uma resposta",
        "no whatsapp", "no email", "no e-mail", "na nota", "nas notas", "no obsidian", "para ", "pra ", "sobre ",
    ];
    if NOT.iter().any(|n| t.starts_with(n)) {
        return None;
    }
    Some(text)
}

/// Types the dictated text into the window the user was in (never into the
/// Jarvis itself: then the program right behind it).
pub fn dictate(text: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let fg = win::foreground();
        let list = win::windows();
        // win::windows() leaves the Jarvis out: the foreground one if it is
        // listed, else the first program behind it.
        let target = list.iter().find(|w| w.hwnd == fg).or_else(|| list.first()).cloned().ok_or("Não achei onde escrever.")?;
        if target.hwnd != fg && !win::focus(target.hwnd) {
            return Err(format!("O Windows não deixou voltar para \"{}\". Clique onde quer escrever e peça de novo.", target.title));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        win::type_text(text);
        Ok(format!("Escrevi em {}.", target.title))
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("Disponível só no Windows.".into())
    }
}

/// The open programs' windows, for "o que está aberto".
pub fn list() -> String {
    #[cfg(windows)]
    {
        let fg = win::foreground();
        win::windows()
            .into_iter()
            .take(25)
            .map(|w| format!("{}{} ({})", if w.hwnd == fg { "[em foco] " } else { "" }, w.title, w.exe))
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

// ------------------------------------------------------------- files

fn expand(path: &str) -> PathBuf {
    let p = path.trim().trim_matches('"');
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let p = if let Some(rest) = p.strip_prefix("~") { format!("{home}{rest}") } else { p.replace("%USERPROFILE%", &home) };
    PathBuf::from(p)
}

/// Folders the Jarvis never writes into.
fn protected(path: &Path) -> bool {
    let p = fold(&path.to_string_lossy()).replace('/', "\\");
    ["c:\\windows", "c:\\program files", "c:\\programdata", "\\appdata\\local\\microsoft", "\\.ssh", "\\.tauri"].iter().any(|bad| p.starts_with(bad) || p.contains(bad))
}

/// A text file's content (the start, for big ones).
pub fn read_file(path: &str) -> Result<String, String> {
    let p = expand(path);
    let bytes = std::fs::read(&p).map_err(|e| format!("Não consegui ler {}: {e}", p.display()))?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!("{} não é um arquivo de texto (Word, PDF, imagem…). Abra com file_open e use app_read para ver o que aparece na janela.", p.display()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let total = text.chars().count();
    let shown: String = text.chars().take(15_000).collect();
    Ok(if total > 15_000 { format!("{shown}\n… (arquivo com {total} caracteres; mostrei o começo)") } else { shown })
}

fn backup(p: &Path) -> Result<Option<PathBuf>, String> {
    if !p.exists() {
        return Ok(None);
    }
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("OpenJarvis").join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "arquivo".into());
    let to = dir.join(format!("{stamp}_{name}"));
    std::fs::copy(p, &to).map_err(|e| format!("Não consegui guardar a cópia de segurança: {e}"))?;
    Ok(Some(to))
}

/// Writes a text file: "overwrite" (all of it), "append" (at the end) or
/// "replace" (find → replace). The old version is kept in backups first.
pub fn write_file(path: &str, mode: &str, content: &str, find: &str, replace: &str) -> Result<String, String> {
    let p = expand(path);
    if p.as_os_str().is_empty() || !p.is_absolute() {
        return Err("Preciso do caminho completo do arquivo (use file_search).".into());
    }
    if protected(&p) {
        return Err(format!("Não mexo em arquivos de sistema ({}).", p.display()));
    }
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            return Err(format!("A pasta {} não existe.", parent.display()));
        }
    }
    let old = if p.exists() { Some(read_file(&p.to_string_lossy()).and_then(|_| std::fs::read_to_string(&p).map_err(|e| e.to_string()))?) } else { None };
    let new = match mode.trim() {
        "append" => format!("{}{}{}", old.clone().unwrap_or_default(), if old.as_deref().is_some_and(|o| !o.is_empty() && !o.ends_with('\n')) { "\n" } else { "" }, content),
        "replace" => {
            let Some(o) = old.clone() else { return Err("O arquivo não existe.".into()) };
            if find.is_empty() {
                return Err("Diga o trecho a trocar (find).".into());
            }
            let n = o.matches(find).count();
            if n == 0 {
                return Err(format!("Não achei \"{find}\" no arquivo. Leia com file_read e use o trecho exato."));
            }
            o.replace(find, replace)
        }
        _ => content.to_string(),
    };
    let saved = backup(&p)?;
    std::fs::write(&p, new.as_bytes()).map_err(|e| format!("Não consegui salvar {}: {e}", p.display()))?;
    Ok(match saved {
        Some(b) => format!("Salvei {}. A versão anterior ficou em {}.", p.display(), b.display()),
        None => format!("Criei {}.", p.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_names_find_their_windows() {
        let w = wanted("Calculadora");
        assert_eq!(matches("Calculadora", "ApplicationFrameHost", &w), 2);
        assert_eq!(matches("Sem título - Bloco de Notas", "Notepad", &wanted("notepad")), 2);
        assert_eq!(matches("relatorio.txt - Notepad", "notepad", &wanted("bloco de notas")), 2);
        assert_eq!(matches("Spotify Premium", "Spotify", &w), 0);
    }

    #[test]
    fn dictation_is_told_apart() {
        assert_eq!(dictation("Escreve: bom dia a todos.").as_deref(), Some("bom dia a todos."));
        assert_eq!(dictation("Jarvis, escreve aí Reunião às 15h").as_deref(), Some("Reunião às 15h"));
        assert_eq!(dictation("digita 42").as_deref(), Some("42"));
        assert_eq!(dictation("Escreve, Olá, tudo bem?").as_deref(), Some("Olá, tudo bem?"));
        assert_eq!(dictation("escreve isso: até amanhã").as_deref(), Some("até amanhã"));
        // Jobs for the AI, not dictation.
        assert_eq!(dictation("escreve um e-mail para a Bia"), None);
        assert_eq!(dictation("escreve uma mensagem no WhatsApp"), None);
        assert_eq!(dictation("escreve para mim um resumo"), None);
        assert_eq!(dictation("escrever"), None);
        assert_eq!(dictation("escrevendo um livro"), None);
        assert_eq!(dictation("toca uma música"), None);
    }

    #[test]
    fn control_names_are_scored() {
        assert_eq!(name_score("Igual a", "igual a"), 4);
        assert!(name_score("Salvar como", "salvar") > name_score("Não salvar", "salvar"));
        assert_eq!(name_score("Abrir", "fechar"), 0);
    }

    #[test]
    fn system_folders_are_off_limits() {
        assert!(protected(Path::new("C:\\Windows\\System32\\drivers\\etc\\hosts")));
        assert!(protected(Path::new("C:\\Users\\x\\.tauri\\key")));
        assert!(!protected(Path::new("D:\\Projetos\\notas.txt")));
    }

    #[test]
    fn files_are_edited_with_a_backup() {
        let dir = std::env::temp_dir().join(format!("jarvis-pc-control-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("lista.txt");
        let path = f.to_string_lossy().to_string();
        assert!(write_file(&path, "overwrite", "pão\nleite", "", "").unwrap().starts_with("Criei"));
        write_file(&path, "append", "café", "", "").unwrap();
        let msg = write_file(&path, "replace", "", "leite", "manteiga").unwrap();
        assert!(msg.contains("versão anterior"));
        assert_eq!(read_file(&path).unwrap(), "pão\nmanteiga\ncafé");
        assert!(write_file(&path, "replace", "", "queijo", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, windows))]
mod live {
    /// Opens the Calculator, does a sum, reads the display. Run by hand:
    /// cargo test --lib pc_control::live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn calculator_on_this_pc() {
        crate::jarvis_pc::jarvis_open_target("calc".into()).unwrap();
        println!("type: {:?}", super::type_into("calculadora", "125*8=", "", ""));
        let r = super::read("calculadora").unwrap();
        println!("{r}");
        assert!(r.contains("1.000") || r.contains("1,000") || r.contains("1000"));
        println!("click: {:?}", super::click("calculadora", "Limpar"));
        // Dictation goes to the window in front.
        println!("ditado: {:?}", super::dictate("7*6="));
        let r = super::read("calculadora").unwrap();
        println!("{}", r.lines().find(|l| l.contains("exibição")).unwrap_or("?"));
        println!("{:?}", super::type_into("calc", "", "alt+f4", ""));
    }
}
