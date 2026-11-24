//! WhatsApp inside Jarvis. WhatsApp Web runs in a hidden window of the app
//! itself (WebView2, with a data folder of its own), driven through WebView2's
//! DevTools protocol — no external browser. The notch's WhatsApp tab shows the
//! QR code, the chats and the messages; the brain can send (after Permitir),
//! read a chat and list the unread ones, only when the user asks.
//!
//! Nothing here talks to a server of its own: it is the user's WhatsApp Web,
//! driven like a person would (search, click, type, Enter).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const URL: &str = "https://web.whatsapp.com/";

/// The error when the QR code must be scanned first.
pub const NEED_LOGIN: &str =
    "O WhatsApp ainda não está conectado ao Jarvis. Abra a aba WhatsApp do notch e escaneie o QR code (no celular: Aparelhos conectados → Conectar um aparelho).";

static APP: OnceLock<AppHandle> = OnceLock::new();
const LABEL: &str = "whatsapp";

/// One request at a time: the view has a single search box and chat pane.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ── The hidden view ──────────────────────────────────────────────────────────

/// The hidden WhatsApp Web view, driven over WebView2's DevTools protocol.
struct Tab {
    win: tauri::WebviewWindow,
}

impl Tab {
    /// The view, created (hidden) on first use.
    async fn get() -> Result<Self, String> {
        let app = APP.get().ok_or("O Jarvis ainda está iniciando.")?;
        if let Some(win) = app.get_webview_window(LABEL) {
            return Ok(Self { win });
        }
        let url = WebviewUrl::External(URL.parse().map_err(|e| format!("{e}"))?);
        let dir = crate::platform::local_dir().join("whatsapp-web");
        let _ = std::fs::create_dir_all(&dir);
        let win = WebviewWindowBuilder::new(app, LABEL, url)
            .title("WhatsApp — Jarvis")
            .visible(false)
            .skip_taskbar(true)
            .inner_size(1100.0, 820.0)
            // Its own WebView2 data: the WhatsApp session lives here, apart
            // from the notch's (one folder with different options fails).
            .data_directory(dir)
            .build()
            .map_err(|e| format!("Não consegui iniciar o WhatsApp no Jarvis: {e}"))?;
        // Calls need the microphone (and the camera): with the window hidden
        // there is nowhere to show WebView2's permission prompt, the request
        // hangs and the call dies. Granted here, for this page only.
        #[cfg(windows)]
        allow_call_media(&win);
        #[cfg(windows)]
        keep_page_visible(&win);
        // Shown for a call; closing it only hides it again (the session stays).
        let hide = win.clone();
        win.on_window_event(move |e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = hide.hide();
                let _ = hide.set_skip_taskbar(true);
                #[cfg(windows)]
                keep_page_visible(&hide);
            }
        });
        crate::log::line("whatsapp: WhatsApp Web iniciado dentro do Jarvis");
        Ok(Self { win })
    }

    /// One DevTools protocol call on the view.
    async fn cdp(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        let (m, p) = (method.to_string(), params.to_string());
        self.win
            .with_webview(move |wv| {
                #[cfg(windows)]
                unsafe {
                    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
                    use wv2_core::HSTRING;
                    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |res, json| {
                        let _ = tx.send(res.map(|_| json).map_err(|e| e.message().to_string()));
                        Ok(())
                    }));
                    if let Ok(core) = wv.controller().CoreWebView2() {
                        let _ = core.CallDevToolsProtocolMethod(&HSTRING::from(m), &HSTRING::from(p), &handler);
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = (wv, m, p);
                    let _ = tx.send(Err("Só no Windows.".into()));
                }
            })
            .map_err(|e| e.to_string())?;
        let json = tokio::time::timeout(Duration::from_secs(20), rx)
            .await
            .map_err(|_| format!("{method}: o WhatsApp demorou demais"))?
            .map_err(|_| format!("{method}: o WhatsApp não respondeu"))??;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }

    async fn eval(&mut self, js: &str) -> Result<Value, String> {
        let r = self
            .cdp("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "awaitPromise": true, "userGesture": true }))
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("erro de script");
            return Err(format!("Erro no WhatsApp: {msg}"));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Types into the focused field, as a paste would (emoji and accents included).
    async fn insert_text(&mut self, text: &str) -> Result<(), String> {
        self.cdp("Input.insertText", json!({ "text": text })).await.map(|_| ())
    }

    async fn enter(&mut self) -> Result<(), String> {
        let down = json!({ "type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r" });
        self.cdp("Input.dispatchKeyEvent", down).await?;
        self.cdp("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13 }))
            .await?;
        Ok(())
    }

    async fn click_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.cdp("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y })).await?;
        for kind in ["mousePressed", "mouseReleased"] {
            self.cdp("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 })).await?;
        }
        Ok(())
    }

    async fn click(&mut self, at: &Value) -> Result<(), String> {
        self.click_at(at["x"].as_f64().unwrap_or(0.0), at["y"].as_f64().unwrap_or(0.0)).await
    }

    async fn navigate(&mut self, url: &str) -> Result<(), String> {
        self.cdp("Page.navigate", json!({ "url": url })).await.map(|_| ())
    }
}

/// The page counts as on screen even with its window hidden: WhatsApp's call
/// module breaks ("something went wrong") on a page it believes is invisible.
#[cfg(windows)]
fn keep_page_visible(win: &tauri::WebviewWindow) {
    let _ = win.with_webview(|wv| unsafe {
        let _ = wv.controller().SetIsVisible(true);
    });
}

/// Microphone and camera for WhatsApp Web's calls, without a prompt.
#[cfg(windows)]
fn allow_call_media(win: &tauri::WebviewWindow) {
    let result = win.with_webview(|wv| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
            COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        };
        use webview2_com::PermissionRequestedEventHandler;
        let Ok(core) = wv.controller().CoreWebView2() else { return };
        let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut uri = wv2_core::PWSTR::null();
            args.Uri(&mut uri)?;
            let from_whatsapp = webview2_com::take_pwstr(uri).starts_with(URL);
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            if from_whatsapp && (kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA) {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                crate::log::line("whatsapp: microfone/câmera liberados para a chamada");
            }
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(e) = core.add_PermissionRequested(&handler, &mut token) {
            crate::log::line(format!("whatsapp: não consegui liberar o microfone: {e}"));
        }
    });
    if let Err(e) = result {
        crate::log::line(format!("whatsapp: permissões: {e}"));
    }
}

/// Called once at startup. If WhatsApp was linked before, it loads in the
/// background a little later, so the first request is quick.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    spawn_call_watch();
    spawn_message_watch();
    // A linked account loads with Jarvis (a few seconds after start, so the
    // notch comes up first): calls, messages and the tab are ready when asked.
    if linked_marker().exists() {
        tauri::async_runtime::spawn(async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if let Ok(mut tab) = Tab::get().await {
                let state = wait_state(&mut tab, Duration::from_secs(90)).await;
                let vis = tab.eval("document.visibilityState").await.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
                // Calls need the microphone: can this page open it?
                let mic = tab
                    .eval(r#"(async () => {
  let perm = '?';
  try { perm = (await navigator.permissions.query({ name: 'microphone' })).state; } catch (e) { perm = 'erro'; }
  let gum = '';
  try {
    const s = await Promise.race([navigator.mediaDevices.getUserMedia({ audio: true }), new Promise((_, no) => setTimeout(() => no(new Error('sem resposta em 8 s')), 8000))]);
    gum = 'ok (' + s.getAudioTracks().map(t => t.label).join(', ') + ')';
    s.getTracks().forEach(t => t.stop());
  } catch (e) { gum = 'falhou: ' + (e.name || '') + ' ' + (e.message || e); }
  let mics = '?';
  try { mics = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === 'audioinput').length; } catch (e) {}
  return 'permissão=' + perm + ' | abrir=' + gum + ' | microfones=' + mics;
})()"#)
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "não consegui testar".into());
                crate::log::line(format!("whatsapp: microfone: {mic}"));
                crate::log::line(format!(
                    "whatsapp: {} (página: {vis})",
                    if state == "ready" { "pronto" } else { "ainda não conectado (abra a aba WhatsApp)" }
                ));
            }
        });
    }
}

fn linked_marker() -> std::path::PathBuf {
    crate::platform::local_dir().join("whatsapp-web").join("linked")
}

// ── Page state ───────────────────────────────────────────────────────────────

const STATE_JS: &str = r#"(() => {
  if (document.querySelector('#pane-side')) return 'ready';
  if (document.querySelector('canvas[aria-label], div[data-ref], [data-testid="qrcode"]')) return 'qr';
  const t = (document.body?.innerText || '').toLowerCase();
  if (/escaneie|scan the qr|conectar com n[uú]mero|link with phone|aparelhos conectados|linked devices/.test(t)) return 'qr';
  return 'loading';
})()"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaState {
    /// "ready" | "qr" | "loading"
    state: String,
    /// The QR code to scan (a data: URL), while not linked.
    qr: Option<String>,
}

/// Linked? And the QR code when not (for the notch tab).
pub async fn state() -> Result<WaState, String> {
    let mut tab = Tab::get().await?;
    let state = tab.eval(STATE_JS).await.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "loading".into());
    let qr = if state == "qr" {
        tab.eval(r#"(() => { const c = document.querySelector('canvas'); try { return c ? c.toDataURL('image/png') : null; } catch { return null; } })()"#)
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
    } else {
        None
    };
    if state == "ready" && !linked_marker().exists() {
        let _ = std::fs::write(linked_marker(), b"1");
        crate::log::line("whatsapp: conectado");
    }
    Ok(WaState { state, qr })
}

/// Waits until the page is linked (or clearly needs the QR code).
async fn wait_state(tab: &mut Tab, max: Duration) -> String {
    let start = Instant::now();
    loop {
        let s = tab.eval(STATE_JS).await.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        if s == "ready" || (s == "qr" && start.elapsed() > Duration::from_secs(3)) || start.elapsed() > max {
            return if s.is_empty() { "loading".into() } else { s };
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// The view, linked and loaded — or NEED_LOGIN.
async fn ready_tab() -> Result<Tab, String> {
    let mut tab = Tab::get().await?;
    match wait_state(&mut tab, Duration::from_secs(40)).await.as_str() {
        "ready" => Ok(tab),
        "qr" => Err(NEED_LOGIN.into()),
        _ => Err("O WhatsApp ainda está carregando. Tente de novo em instantes.".into()),
    }
}

/// Polls `js` until it returns something truthy.
async fn wait_for(tab: &mut Tab, js: &str, max_ms: u64) -> Option<Value> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(max_ms) {
        if let Ok(v) = tab.eval(js).await {
            let truthy = match &v {
                Value::Null => false,
                Value::Bool(b) => *b,
                Value::String(s) => !s.is_empty(),
                _ => true,
            };
            if truthy {
                return Some(v);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

// ── Opening a chat ───────────────────────────────────────────────────────────

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' => 'e',
            'í' | 'ì' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// "eu", "mim", "pra mim"…: the user's own chat ("Você" / message yourself).
pub fn is_self(to: &str) -> bool {
    let t = fold(to);
    let t = t.trim_start_matches("para ").trim_start_matches("pra ").trim();
    matches!(t, "eu" | "mim" | "me" | "eu mesmo" | "eu mesma" | "comigo" | "meu numero" | "meu whatsapp" | "voce" | "voce mesmo" | "self")
}

/// A phone number in `to` (10+ digits), opened directly.
pub fn phone_digits(to: &str) -> Option<String> {
    let only_phone_chars = to.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c));
    let digits: String = to.chars().filter(|c| c.is_ascii_digit()).collect();
    (only_phone_chars && digits.len() >= 10).then_some(digits)
}

const TITLE_JS: &str = r#"(() => {
  const h = document.querySelector('#main header');
  if (!h || !document.querySelector('#main footer [contenteditable="true"]')) return '';
  // The name as shown, not a tooltip ("clique para mostrar os dados do contato")
  // nor the status line ("online", "digitando…").
  const skip = /clique|click|dados d|contact info|online|digitando|typing|visto por|last seen|gravando/i;
  for (const e of h.querySelectorAll('span[dir="auto"], span[title], div[dir="auto"]')) {
    const t = (e.innerText || '').trim();
    if (t && !skip.test(t)) return t;
  }
  return 'conversa';
})()"#;

/// Is this message the user's own? WhatsApp has used several markers over
/// time; any of them counts.
const MINE_JS: &str = r#"(e => {
  const id = e.closest('[data-id]')?.getAttribute('data-id') || '';
  if (id.startsWith('true_')) return true;
  if (id.startsWith('false_')) return false;
  if (e.closest('.message-out')) return true;
  if (e.closest('.message-in')) return false;
  const row = e.closest('[role="row"], .focusable-list-item') || e.parentElement;
  if (row?.querySelector('[data-icon^="msg-check"], [data-icon^="msg-dblcheck"], [data-icon="msg-time"], [data-icon^="tail-out"]')) return true;
  // Markup-free: the user's own bubbles sit on the right half of the chat.
  const main = document.querySelector('#main')?.getBoundingClientRect();
  const b = e.getBoundingClientRect();
  return !!main && b.width > 0 && (b.left + b.right) / 2 > (main.left + main.right) / 2;
})"#;

const SEARCH_BOX_JS: &str = r#"(() => {
  const side = document.querySelector('#side');
  const b = side && (side.querySelector('[contenteditable="true"]') || side.querySelector('input'));
  if (!b) return null;
  const r = b.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})()"#;

const CLEAR_FOCUSED_JS: &str = r#"(() => {
  const a = document.activeElement;
  if (!a) return false;
  if (a.isContentEditable) { document.execCommand('selectAll', false); document.execCommand('delete', false); }
  else if ('value' in a) { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(a, ''); a.dispatchEvent(new Event('input', { bubbles: true })); }
  return true;
})()"#;

/// Leaves the chat list as it was (no search text left behind).
async fn clear_search(tab: &mut Tab) {
    let _ = tab
        .eval(
            r#"(() => {
  const side = document.querySelector('#side');
  const b = side && (side.querySelector('[contenteditable="true"]') || side.querySelector('input'));
  if (!b || !(b.innerText || b.value || '').trim()) return false;
  b.focus();
  if (b.isContentEditable) { document.execCommand('selectAll', false); document.execCommand('delete', false); }
  else { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(b, ''); b.dispatchEvent(new Event('input', { bubbles: true })); }
  return true;
})()"#,
        )
        .await;
}

/// Opens the chat with `to` (a contact or group name, a number, or the user).
/// Returns the chat's title as WhatsApp shows it.
async fn open_chat(tab: &mut Tab, to: &str) -> Result<String, String> {
    let to = resolve_contact(to);
    let to = to.as_str();
    // Already open?
    if let Ok(Value::String(t)) = tab.eval(TITLE_JS).await {
        if !t.is_empty() && fold(&t) == fold(to) {
            return Ok(t);
        }
    }
    let number = if is_self(to) {
        // The linked account's own number, as WhatsApp Web stores it.
        let wid = tab
            .eval(r#"(() => { const v = localStorage.getItem('last-wid-md') || localStorage.getItem('last-wid') || ''; return v.replace(/[^0-9:@]/g, '').split(/[:@]/)[0]; })()"#)
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        Some(wid).filter(|d| d.len() >= 10)
    } else {
        phone_digits(to)
    };

    if let Some(d) = number {
        tab.navigate(&format!("{URL}send?phone={d}")).await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let title = wait_for(tab, TITLE_JS, 25_000).await.ok_or("Não consegui abrir essa conversa no WhatsApp (o número tem WhatsApp?).")?;
        return Ok(if is_self(to) { "você mesmo".into() } else { title.as_str().unwrap_or(to).to_string() });
    }

    // In the chat list already (the usual case from the notch): a click.
    let direct = format!(
        r#"(() => {{
  const fold = s => (s || '').normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().trim();
  const want = fold({want});
  const pane = document.querySelector('#pane-side');
  const top = pane ? pane.getBoundingClientRect().top : 0;
  for (const r of document.querySelectorAll('#pane-side [role="listitem"], #pane-side [role="row"]')) {{
    const t = r.querySelector('span[title]');
    const b = r.getBoundingClientRect();
    if (t && fold(t.getAttribute('title')) === want && b.height > 20 && b.top >= top && b.bottom <= innerHeight) return {{ x: b.left + b.width / 2, y: b.top + b.height / 2 }};
  }}
  return null;
}})()"#,
        want = serde_json::to_string(to.trim()).unwrap_or_default()
    );
    if let Some(at) = point(tab, &direct).await {
        tab.click(&at).await?;
        if let Some(Value::String(t)) = wait_for(tab, TITLE_JS, 4_000).await {
            if fold(&t) == fold(to) {
                return Ok(t);
            }
        }
    }

    // By name: the search box of the chat list.
    let at = wait_for(tab, SEARCH_BOX_JS, 5_000).await.ok_or("Não achei a busca de conversas do WhatsApp.")?;
    tab.click(&at).await?;
    tab.eval(CLEAR_FOCUSED_JS).await?;
    tab.insert_text(to.trim()).await?;

    let pick_js = format!(
        r#"(() => {{
  const fold = s => (s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();
  const want = fold({want});
  const items = [];
  for (const r of document.querySelectorAll('#pane-side [role="listitem"], #pane-side [role="row"]')) {{
    const t = r.querySelector('span[title]');
    const rect = r.getBoundingClientRect();
    if (!t || rect.height < 20) continue;
    const title = t.getAttribute('title');
    if (items.some(i => i.title === title)) continue;
    items.push({{ title, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }});
  }}
  const f = i => fold(i.title);
  const pick = items.find(i => f(i) === want) || items.find(i => f(i).startsWith(want)) || items.find(i => f(i).includes(want))
    || items.find(i => want.split(' ').every(w => f(i).includes(w)));
  return {{ pick: pick || null, titles: items.map(i => i.title).slice(0, 8) }};
}})()"#,
        want = serde_json::to_string(to.trim()).unwrap_or_default()
    );
    // The results show up within a second or so: no fixed wait.
    let mut found = Value::Null;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        found = tab.eval(&pick_js).await?;
        if found.get("pick").is_some_and(|p| !p.is_null()) {
            break;
        }
    }
    let Some(pick) = found.get("pick").filter(|p| !p.is_null()) else {
        clear_search(tab).await;
        let seen: Vec<String> = found["titles"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
        return Err(if seen.is_empty() {
            format!("Não achei \"{to}\" nas conversas nem nos contatos do WhatsApp.")
        } else {
            format!("Não achei \"{to}\" exatamente. Parecidos: {}.", seen.join(", "))
        });
    };
    let pick = pick.clone();
    tab.click(&pick).await?;
    let title = wait_for(tab, TITLE_JS, 8_000).await.ok_or("Não consegui abrir a conversa.")?;
    clear_search(tab).await;
    Ok(title.as_str().unwrap_or(to).to_string())
}

// ── What the notch and the brain use ─────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WaChat {
    title: String,
    last: String,
    time: String,
    unread: u32,
    /// The contact's or group's photo (an https URL), when it has one.
    avatar: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WaMessage {
    /// Its handle for media actions while the chat is open ("m12").
    id: String,
    /// "text" | "audio" | "image" | "video" | "document" | "sticker"
    kind: String,
    when: String,
    from: String,
    text: String,
    mine: bool,
    /// Voice notes: "0:12".
    duration: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaThread {
    title: String,
    messages: Vec<WaMessage>,
    /// The chat's photo, from its header.
    avatar: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaChatPage {
    chats: Vec<WaChat>,
    /// More chats exist below the ones returned.
    more: bool,
}

/// WhatsApp only renders the chats in view: the list is scrolled page by page
/// and collected until `__LIMIT__` chats (or the end), then put back on top.
const CHATS_SCROLL_JS: &str = r#"(async () => {
  const LIMIT = __LIMIT__;
  const pane = document.querySelector('#pane-side');
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const seen = new Map();
  const grab = () => {
    for (const c of (__ROWS__)) if (!seen.has(c.title)) seen.set(c.title, c);
  };
  if (!pane) return { chats: (__ROWS__).slice(0, LIMIT), more: false };
  pane.scrollTop = 0;
  await sleep(120);
  grab();
  let stall = 0, atEnd = false;
  while (seen.size < LIMIT + 1 && stall < 4) {
    const before = seen.size;
    pane.scrollTop += Math.max(200, pane.clientHeight * 0.85);
    await sleep(260);
    grab();
    atEnd = pane.scrollTop + pane.clientHeight >= pane.scrollHeight - 4;
    stall = seen.size === before ? stall + 1 : 0;
    if (atEnd && seen.size === before) break;
  }
  pane.scrollTop = 0;
  const all = [...seen.values()];
  return { chats: all.slice(0, LIMIT), more: all.length > LIMIT || !atEnd };
})()"#;

const CHATS_JS: &str = r#"(() => {
  const out = [];
  for (const r of document.querySelectorAll('#pane-side [role="listitem"], #pane-side [role="row"]')) {
    const titles = [...r.querySelectorAll('span[title]')].map(s => s.getAttribute('title'));
    const title = titles[0];
    if (!title || out.some(o => o.title === title)) continue;
    const lines = (r.innerText || '').split('\n').map(s => s.trim()).filter(Boolean);
    const time = lines.find(l => /^(\d{1,2}:\d{2}|ontem|anteontem|yesterday|\d{1,2}\/\d{1,2}\/\d{2,4}|segunda|terça|quarta|quinta|sexta|sábado|domingo)/i.test(l)) || '';
    const badge = [...r.querySelectorAll('span[aria-label]')].find(s => /n[aã]o lida|unread/i.test(s.getAttribute('aria-label')));
    const unread = badge ? parseInt((badge.innerText || badge.getAttribute('aria-label')).replace(/\D/g, '') || '1', 10) : 0;
    const last = titles.length > 1 ? titles[titles.length - 1] : (lines.filter(l => l !== title && l !== time && !/^\d+$/.test(l)).pop() || '');
    const rect = r.getBoundingClientRect();
    const img = [...r.querySelectorAll('img')].find(im => /^https:/.test(im.src) && im.getBoundingClientRect().width >= 24);
    out.push({ title, last, time, unread, avatar: img ? img.src : '', top: rect.top });
  }
  return out.sort((a, b) => a.top - b.top).map(({ top, ...c }) => c);
})()"#;

/// The last messages of the open chat, of every kind. Each one gets a
/// `data-jv-msg` handle for the media actions (play, image).
fn messages_js(n: usize) -> String {
    format!(
        r#"(() => {{
  const isMine = {mine_js};
  const main = document.querySelector('#main');
  if (!main) return [];
  // One element per message: its row, else the text block itself.
  let rows = [...main.querySelectorAll('[role="row"]')].filter(r => r.querySelector('[data-pre-plain-text], img, [data-icon], audio'));
  if (!rows.length) rows = [...main.querySelectorAll('[data-pre-plain-text]')];
  rows = rows.slice(-{n});
  const hhmm = /^\d{{1,2}}:\d{{2}}$/;
  return rows.map((r, i) => {{
    const id = 'm' + i;
    r.setAttribute('data-jv-msg', id);
    const pre = r.querySelector('[data-pre-plain-text]');
    const meta = pre ? (pre.getAttribute('data-pre-plain-text') || '').match(/^\[(.*?)\]\s*(.*?):\s*$/) : null;
    const mine = isMine(pre || r);
    const icon = n => r.querySelector('[data-icon*="' + n + '"]');
    const label = re => [...r.querySelectorAll('[aria-label]')].some(e => re.test(e.getAttribute('aria-label')));
    const bigImg = [...r.querySelectorAll('img')].find(im => {{ const b = im.getBoundingClientRect(); return b.width > 70 && b.height > 70 && !/emoji/i.test(im.className); }});
    let kind = 'text';
    if (icon('audio-play') || icon('audio-pause') || icon('ptt') || label(/mensagem de voz|voice message|reproduzir áudio|play audio/i)) kind = 'audio';
    else if (icon('media-play') || icon('video')) kind = 'video';
    else if (bigImg && (bigImg.closest('[role="button"]') || r.querySelector('[data-icon*="media"]')) && !icon('sticker')) kind = 'image';
    else if (icon('document') || icon('doc-') || label(/documento|document/i)) kind = 'document';
    else if (icon('sticker') || (bigImg && !pre)) kind = 'sticker';
    if (!pre && kind === 'text') kind = 'other';
    const spans = [...r.querySelectorAll('span')].map(s => (s.innerText || '').trim());
    const times = spans.filter(t => hhmm.test(t));
    const when = meta ? meta[1] : (times[times.length - 1] || '');
    const text = pre ? ((pre.querySelector('.selectable-text') || pre).innerText || '').trim() : '';
    return {{ id, kind, when, from: mine ? 'Você' : (meta ? meta[2] : ''), text, mine, duration: kind === 'audio' ? (times[0] || '') : '' }};
  }}).filter(m => m.kind !== 'other' || m.text);
}})()"#,
        n = n.clamp(1, 600),
        mine_js = MINE_JS
    )
}

/// The chat list, top first (what WhatsApp has loaded, ~20 chats).
pub async fn chats() -> Result<Vec<WaChat>, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    clear_search(&mut tab).await;
    let v = tab.eval(CHATS_JS).await?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// Selects one of WhatsApp's own list filters: "all" | "unread" | "favorites" | "groups".
async fn set_filter(tab: &mut Tab, filter: &str) {
    let re = match filter {
        "unread" => "^(n[aã]o lidas|unread)$",
        "favorites" => "^(favoritas|favorites)$",
        "groups" => "^(grupos|groups)$",
        _ => "^(tudo|todas|all)$",
    };
    let js = format!(
        r#"(() => {{
  const chips = [...document.querySelectorAll('#side button, #side [role="tab"]')].filter(b => b.getBoundingClientRect().width > 0);
  const c = chips.find(b => new RegExp({re:?}, 'i').test((b.innerText || '').trim().split('\n')[0].trim()));
  if (!c) return null;
  if (c.getAttribute('aria-pressed') === 'true' || c.getAttribute('aria-selected') === 'true') return {{ already: true }};
  const r = c.getBoundingClientRect();
  return {{ x: r.left + r.width / 2, y: r.top + r.height / 2 }};
}})()"#
    );
    if let Some(at) = point(tab, &js).await {
        let _ = tab.click(&at).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
}

/// Chats and contacts matching `query` (WhatsApp's own search).
pub async fn search_chats(query: &str) -> Result<Vec<WaChat>, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    let at = wait_for(&mut tab, SEARCH_BOX_JS, 5_000).await.ok_or("Não achei a busca de conversas do WhatsApp.")?;
    tab.click(&at).await?;
    tab.eval(CLEAR_FOCUSED_JS).await?;
    tab.insert_text(query.trim()).await?;
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let v = tab.eval(CHATS_JS).await?;
    clear_search(&mut tab).await;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The first `limit` chats (of a filter), scrolling WhatsApp's list to reach them.
pub async fn chat_page_filtered(limit: usize, filter: &str) -> Result<WaChatPage, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    clear_search(&mut tab).await;
    set_filter(&mut tab, filter).await;
    // A quick look (the periodic refresh): only the chats on screen, no scrolling.
    if limit <= 20 {
        let v = tab.eval(CHATS_JS).await?;
        let chats: Vec<WaChat> = serde_json::from_value(v).unwrap_or_default();
        return Ok(WaChatPage { more: true, chats });
    }
    let rows = CHATS_JS.trim_end_matches("()").trim_end();
    let js = CHATS_SCROLL_JS.replace("__LIMIT__", &limit.clamp(10, 1000).to_string()).replace("__ROWS__", &format!("{rows}()"));
    let v = tab.eval(&js).await?;
    let chats: Vec<WaChat> = serde_json::from_value(v["chats"].clone()).unwrap_or_default();
    crate::log::line(format!("whatsapp: {} conversas carregadas", chats.len()));
    Ok(WaChatPage { chats, more: v["more"].as_bool().unwrap_or(false) })
}

/// A point to click on, from a script returning {x, y} (or null).
async fn point(tab: &mut Tab, js: &str) -> Option<Value> {
    tab.eval(js).await.ok().filter(|v| v.get("x").is_some())
}

fn in_msg(id: &str, inner: &str) -> String {
    format!(
        r#"(() => {{
  const r = document.querySelector('#main [data-jv-msg="{id}"]');
  if (!r) return null;
  {inner}
}})()"#,
        id = id.replace(['"', '\'', '\\'], "")
    )
}

const CENTER: &str = "const b = el.getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 };";

/// Plays (or pauses) a voice note in the open chat: WhatsApp plays it on the
/// PC's speakers. Returns "playing" or "paused".
pub async fn play_audio(chat: &str, id: &str) -> Result<String, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    open_chat(&mut tab, chat).await?;
    let js = in_msg(
        id,
        &format!(
            r#"const el = r.querySelector('[data-icon*="audio-play"], [data-icon*="audio-pause"]')?.closest('button,[role="button"]')
    || r.querySelector('button[aria-label*="eproduzir" i], button[aria-label*="lay" i], button[aria-label*="ausar" i], button[aria-label*="ause" i]');
  if (!el) return null;
  el.scrollIntoView({{ block: 'center' }});
  {CENTER}"#
        ),
    );
    let at = point(&mut tab, &js).await.ok_or("Não achei o áudio nessa conversa (recarregue a conversa).")?;
    tab.click(&at).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let state = tab
        .eval(&in_msg(id, r#"return r.querySelector('[data-icon*="audio-pause"]') ? 'playing' : 'paused';"#))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "playing".into());
    Ok(state)
}

/// An image from the open chat, as a data: URL (downloading it first when
/// WhatsApp only shows the blurred preview).
pub async fn image(chat: &str, id: &str) -> Result<String, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    open_chat(&mut tab, chat).await?;
    // Not downloaded yet: its download button first.
    let dl = in_msg(
        id,
        &format!(
            r#"const el = r.querySelector('[data-icon*="download"]')?.closest('button,[role="button"],span');
  if (!el) return null;
  el.scrollIntoView({{ block: 'center' }});
  {CENTER}"#
        ),
    );
    if let Some(at) = point(&mut tab, &dl).await {
        tab.click(&at).await?;
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }
    let grab = in_msg(
        id,
        r#"const im = [...r.querySelectorAll('img')].filter(im => im.getBoundingClientRect().width > 70).sort((a, b) => (b.naturalWidth || 0) - (a.naturalWidth || 0))[0];
  if (!im) return null;
  return fetch(im.src).then(res => res.blob()).then(b => new Promise(ok => { const fr = new FileReader(); fr.onload = () => ok(fr.result); fr.readAsDataURL(b); }));"#,
    );
    for _ in 0..8 {
        if let Ok(Value::String(url)) = tab.eval(&grab).await {
            if url.starts_with("data:image") && url.len() > 2000 {
                return Ok(url);
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    Err("Não consegui carregar essa imagem.".into())
}

/// Saves a data: URL image in Downloads\WhatsApp (Jarvis). Returns the path.
pub fn save_image(data_url: &str, name_hint: &str) -> Result<String, String> {
    let (head, b64) = data_url.split_once(',').ok_or("Imagem inválida.")?;
    let ext = if head.contains("png") { "png" } else if head.contains("webp") { "webp" } else { "jpg" };
    let bytes = crate::jarvis_brain::decode_base64_pub(b64)?;
    let downloads = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_default().join("Downloads").join("WhatsApp (Jarvis)");
    std::fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
    let safe: String = name_hint.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let path = downloads.join(format!("{} {stamp}.{ext}", safe.trim()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Voice note with WhatsApp's own recorder: "start" presses the microphone in
/// the open chat, "send" sends it, "cancel" throws it away.
pub async fn voice_note(chat: &str, action: &str) -> Result<String, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    open_chat(&mut tab, chat).await?;
    let find = |sel: &str| {
        format!(
            r#"(() => {{
  const el = [...document.querySelectorAll('#main footer {sel}, #main {sel}')].map(e => e.closest('button,[role="button"]') || e).find(e => e.getBoundingClientRect().width > 0);
  if (!el) return null;
  {CENTER}
}})()"#
        )
    };
    match action {
        "start" => {
            // The page may record without a prompt (it is hidden: nobody could answer one).
            let _ = tab
                .cdp("Browser.grantPermissions", json!({ "origin": "https://web.whatsapp.com", "permissions": ["audioCapture"] }))
                .await;
            let at = point(&mut tab, &find(r#"[data-icon*="ptt"], [data-icon*="mic"], button[aria-label*="voz" i], button[aria-label*="voice" i]"#))
                .await
                .ok_or("Não achei o botão de gravar áudio nessa conversa.")?;
            tab.click(&at).await?;
            tokio::time::sleep(Duration::from_millis(700)).await;
            let recording = tab
                .eval(r#"(() => !!document.querySelector('#main footer [data-icon*="send"], #main footer [data-icon*="delete"], #main footer [data-icon*="trash"]'))()"#)
                .await
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if recording {
                crate::log::line("whatsapp: gravando áudio");
                Ok("recording".into())
            } else {
                Err("O WhatsApp não começou a gravar (sem acesso ao microfone?).".into())
            }
        }
        "send" => {
            let at = point(&mut tab, &find(r#"[data-icon*="send"]"#)).await.ok_or("Não achei o botão de enviar o áudio.")?;
            tab.click(&at).await?;
            crate::log::line("whatsapp: áudio enviado");
            Ok("sent".into())
        }
        _ => {
            if let Some(at) = point(&mut tab, &find(r#"[data-icon*="delete"], [data-icon*="trash"], [data-icon*="bin"]"#)).await {
                tab.click(&at).await?;
            }
            Ok("cancelled".into())
        }
    }
}

/// Opens a chat and returns its last `count` messages, oldest first.
pub async fn thread(chat: &str, count: usize) -> Result<WaThread, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    let title = open_chat(&mut tab, chat).await?;
    let _ = wait_for(&mut tab, r#"document.querySelectorAll('#main [role="row"], #main [data-pre-plain-text]').length > 0"#, 2_500).await;
    if count > 30 {
        // Older messages: WhatsApp loads them when its list reaches the top.
        let _ = tab.eval(&OLDER_JS.replace("__WANT__", &count.to_string())).await;
    }
    let v = tab.eval(&messages_js(count)).await?;
    log_structure(&mut tab).await;
    let avatar = tab
        .eval(r#"(() => { const im = document.querySelector('#main header img'); return im && /^https:/.test(im.src) ? im.src : ''; })()"#)
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    Ok(WaThread { title, messages: serde_json::from_value(v).unwrap_or_default(), avatar })
}

/// Scrolls the open chat up until about `__WANT__` messages are loaded (or
/// nothing more comes). Returns how many there are.
const OLDER_JS: &str = r#"(async () => {
  const WANT = __WANT__;
  const count = () => document.querySelectorAll('#main [role="row"]').length || document.querySelectorAll('#main [data-pre-plain-text]').length;
  let box = document.querySelector('#main [role="row"], #main [data-pre-plain-text]');
  while (box && !(box.scrollHeight > box.clientHeight + 10 && /(auto|scroll)/.test(getComputedStyle(box).overflowY))) box = box.parentElement;
  if (!box) return count();
  let stall = 0;
  while (count() < WANT && stall < 4) {
    const before = count();
    box.scrollTop = 0;
    await new Promise(r => setTimeout(r, 800));
    stall = count() === before ? stall + 1 : 0;
  }
  return count();
})()"#;

/// Calls `chat` (voice or video) from WhatsApp Web. The call screen is
/// WhatsApp's own, so its window comes up for the call; closing it hides it.
pub async fn call(chat: &str, video: bool) -> Result<String, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    let title = open_chat(&mut tab, chat).await?;
    // Microphone (and camera) without a prompt nobody could answer.
    let _ = tab
        .cdp("Browser.grantPermissions", json!({ "origin": "https://web.whatsapp.com", "permissions": ["audioCapture", "videoCapture"] }))
        .await;
    let want = if video { "video" } else { "voice" };
    let header_btn = format!(
        r#"(() => {{
  const btns = [...document.querySelectorAll('#main header button, #main header [role="button"]')];
  const label = b => ((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('title') || '') + ' ' + [...b.querySelectorAll('[data-icon]')].map(i => i.getAttribute('data-icon')).join(' ')).toLowerCase();
  const isVideo = b => /v[ií]deo|video/.test(label(b));
  const isCall = b => /liga[cç]|chamada|call|phone|telefon|voice|voz/.test(label(b)) || isVideo(b);
  const pick = '{want}' === 'video' ? btns.find(isVideo) : (btns.find(b => isCall(b) && !isVideo(b)) || btns.find(isCall));
  if (!pick) return {{ labels: btns.map(label).filter(Boolean).slice(0, 12) }};
  const b = pick.getBoundingClientRect();
  return {{ x: b.left + b.width / 2, y: b.top + b.height / 2 }};
}})()"#
    );
    // The call buttons come a moment after the chat opens: wait for them.
    let mut found = tab.eval(&header_btn).await?;
    for _ in 0..24 {
        if found.get("x").is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        found = tab.eval(&header_btn).await?;
    }
    // WhatsApp's call buttons crashed ("something went wrong, refresh"): a
    // reload of the page brings them back.
    let broken = found["labels"].as_array().is_some_and(|l| l.iter().any(|x| x.as_str().is_some_and(|s| s.contains("went wrong") || s.contains("deu errado"))));
    let found = if broken {
        crate::log::line("whatsapp: botões de chamada com erro; recarregando o WhatsApp");
        tab.cdp("Page.reload", json!({})).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if wait_state(&mut tab, Duration::from_secs(45)).await != "ready" {
            return Err("O WhatsApp está recarregando; tente ligar de novo em alguns segundos.".into());
        }
        open_chat(&mut tab, &title).await?;
        let mut f = tab.eval(&header_btn).await?;
        for _ in 0..24 {
            if f.get("x").is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            f = tab.eval(&header_btn).await?;
        }
        f
    } else {
        found
    };
    if found.get("x").is_none() {
        crate::log::line(format!("whatsapp: sem botão de chamada; cabeçalho: {}", found["labels"]));
        return Err("O WhatsApp Web desta conta não mostrou o botão de ligar nessa conversa (as chamadas no navegador ainda estão chegando aos poucos). Ligue pelo celular ou pelo app do WhatsApp.".into());
    }
    // Voice calls stay hidden (the notch has the controls); a video call needs
    // its picture, so WhatsApp's window comes up.
    if video {
        show_window(&tab.win);
    }
    // In case a hide marked the page invisible again.
    #[cfg(windows)]
    keep_page_visible(&tab.win);
    tab.click(&found).await?;
    tokio::time::sleep(Duration::from_millis(900)).await;
    // WhatsApp asks which kind: "Ligar para X — Cancelar / Ligação de voz /
    // Ligação de vídeo". The dialog comes a moment after the click.
    let choose = format!(
        r#"(() => {{
  const dialogs = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"], [data-animate-modal-popup]')].filter(d => d.getBoundingClientRect().width > 0);
  const want = '{want}' === 'video' ? /liga[cç][aã]o de v[ií]deo|chamada de v[ií]deo|video call/i : /liga[cç][aã]o de voz|chamada de voz|voice call/i;
  for (const d of (dialogs.length ? dialogs : [document])) {{
    const it = [...d.querySelectorAll('button, [role="button"], [role="menuitem"], [role="option"]')]
      .filter(e => e.getBoundingClientRect().width > 0)
      .find(e => want.test(((e.innerText || '') + ' ' + (e.getAttribute('aria-label') || '')).trim()));
    if (it) {{ const b = it.getBoundingClientRect(); return {{ x: b.left + b.width / 2, y: b.top + b.height / 2 }}; }}
  }}
  return null;
}})()"#
    );
    let mut chose = false;
    for _ in 0..16 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(at) = point(&mut tab, &choose).await {
            tab.click(&at).await?;
            chose = true;
            break;
        }
    }
    if !chose {
        crate::log::line("whatsapp: o WhatsApp não mostrou a escolha voz/vídeo; seguindo");
    }
    crate::log::line(format!("whatsapp: chamada de {} para {title}", if video { "vídeo" } else { "voz" }));
    // What WhatsApp says right after (errors, "no microphone"…): only its
    // notices and the call box, never the messages.
    let probe = r#"(() => {
  const bits = [];
  for (const e of document.querySelectorAll('[role="dialog"], [role="alert"], [role="alertdialog"], [data-animate-modal-popup]')) {
    const t = (e.innerText || '').replace(/\s+/g, ' ').trim();
    if (t) bits.push('aviso: ' + t.slice(0, 160));
  }
  const hang = [...document.querySelectorAll('button,[role="button"]')].find(b => /encerrar|desligar|end[- ]?call|call-end|hang ?up/i.test((b.getAttribute('aria-label') || '') + ' ' + [...b.querySelectorAll('[data-icon]')].map(i => i.getAttribute('data-icon')).join(' ')));
  if (hang) {
    let box = hang;
    for (let i = 0; i < 6 && box.parentElement; i++) box = box.parentElement;
    bits.push('chamada: ' + (box.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 120));
  }
  bits.push('mic: ' + (navigator.mediaDevices ? 'api ok' : 'sem api') + ', visível: ' + document.visibilityState + ', foco: ' + document.hasFocus());
  return bits.join(' | ');
})()"#;
    // In the background: the reply must not wait for it.
    tauri::async_runtime::spawn(async move {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _busy = BUSY.lock().await;
            if let Ok(mut tab) = Tab::get().await {
                if let Ok(Value::String(s)) = tab.eval(probe).await {
                    crate::log::line(format!("whatsapp: diagnóstico da chamada: {s}"));
                }
            }
        }
    });
    Ok(if video {
        format!("Chamada de vídeo para {title}: a janela do WhatsApp abriu para o vídeo.")
    } else {
        format!("Ligando para {title}… os controles estão na aba WhatsApp do notch.")
    })
}

fn show_window(win: &tauri::WebviewWindow) {
    let _ = win.set_skip_taskbar(false);
    let _ = win.show();
    let _ = win.center();
    let _ = win.set_focus();
}

// ── Calls in the notch ───────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct WaCall {
    /// "none" | "incoming" | "active"
    pub state: String,
    name: String,
    /// "0:33" while connected.
    time: String,
    /// "Chamando…" before the other side answers.
    status: String,
    pub mic_off: bool,
    cam_off: bool,
}

/// WhatsApp's call bar (or incoming-call box): who, how long, mic and camera.
/// Found by its buttons' labels and icons, not by class names.
const CALL_JS: &str = r#"(() => {
  const lab = e => ((e.getAttribute('aria-label') || '') + ' ' + (e.getAttribute('title') || '') + ' ' + (e.getAttribute('data-icon') || '') + ' '
    + [...e.querySelectorAll('[data-icon]')].map(i => i.getAttribute('data-icon')).join(' ')).toLowerCase();
  const visible = b => b.getBoundingClientRect().width > 0;
  const all = [...document.querySelectorAll('button,[role="button"]')].filter(visible);
  const hang = all.find(b => /encerrar|desligar|end[- ]?call|call-end|hang ?up|sair da chamada|leave call/.test(lab(b)));
  const accept = all.find(b => /aceitar|atender|accept|answer/.test(lab(b)));
  const decline = all.find(b => /recusar|rejeitar|decline|reject/.test(lab(b)));
  const anchor = hang || (accept && decline ? accept : null);
  if (!anchor) return { state: 'none' };
  let box = anchor;
  for (let i = 0; i < 8 && box.parentElement; i++) {
    box = box.parentElement;
    if (box.getBoundingClientRect().height > 70) break;
  }
  const texts = [...box.querySelectorAll('span,div')].filter(e => !e.children.length).map(e => (e.innerText || '').trim()).filter(Boolean);
  const time = texts.find(t => /^\d{1,2}:\d{2}(:\d{2})?$/.test(t)) || '';
  const status = texts.find(t => /chamando|tocando|ringing|calling|conectando|connecting|ligando/i.test(t)) || '';
  const name = texts.find(t => t !== time && t !== status && t.length > 1 && t.length < 60) || '';
  const inBox = [...box.querySelectorAll('button,[role="button"]')].filter(visible);
  const mic = inBox.find(b => /microfone|silenciar|mudo|mute|\bmic/.test(lab(b)));
  const cam = inBox.find(b => /c[aâ]mera|camera|v[ií]deo|video/.test(lab(b)));
  return {
    state: hang ? 'active' : 'incoming', name, time, status,
    micOff: mic ? /\bativar (o )?microfone|unmute|ativar som|mic-off|muted|reativar/.test(lab(mic)) : false,
    camOff: cam ? /\bativar (a )?c[aâ]mera|turn on|video-off|camera-off|ligar (a )?c[aâ]mera/.test(lab(cam)) : true,
  };
})()"#;

/// Presses one of the call's buttons: "hangup" | "mute" | "camera" | "accept" | "decline" | "show".
pub async fn call_action(action: &str) -> Result<WaCall, String> {
    let _busy = BUSY.lock().await;
    let mut tab = Tab::get().await?;
    if action == "show" {
        show_window(&tab.win);
        return call_state(&mut tab).await;
    }
    let re = match action {
        "hangup" => r"encerrar|desligar|end[- ]?call|call-end|hang ?up|sair da chamada|leave call",
        "accept" => r"aceitar|atender|accept|answer",
        "decline" => r"recusar|rejeitar|decline|reject",
        "mute" => r"microfone|silenciar|mudo|mute|\bmic",
        "camera" => r"c[aâ]mera|camera|v[ií]deo|video",
        _ => return Err("Ação de chamada desconhecida.".into()),
    };
    let js = format!(
        r#"(() => {{
  const lab = e => ((e.getAttribute('aria-label') || '') + ' ' + (e.getAttribute('title') || '') + ' ' + (e.getAttribute('data-icon') || '') + ' '
    + [...e.querySelectorAll('[data-icon]')].map(i => i.getAttribute('data-icon')).join(' ')).toLowerCase();
  const all = [...document.querySelectorAll('button,[role="button"]')].filter(b => b.getBoundingClientRect().width > 0);
  // Mute / camera: the call's own buttons, near the hang-up one (not the chat's mic).
  const hang = all.find(b => /encerrar|desligar|end[- ]?call|call-end|hang ?up|sair da chamada|leave call/.test(lab(b)));
  let pool = all;
  if (hang && ['mute', 'camera'].includes('{action}')) {{
    let box = hang;
    for (let i = 0; i < 8 && box.parentElement; i++) {{ box = box.parentElement; if (box.getBoundingClientRect().height > 70) break; }}
    pool = [...box.querySelectorAll('button,[role="button"]')];
  }}
  const el = pool.find(b => new RegExp({re:?}).test(lab(b)));
  if (!el) return null;
  const b = el.getBoundingClientRect();
  return {{ x: b.left + b.width / 2, y: b.top + b.height / 2 }};
}})()"#
    );
    let at = point(&mut tab, &js).await.ok_or("Não achei esse botão na chamada.")?;
    tab.click(&at).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    if action == "accept" {
        crate::log::line("whatsapp: chamada atendida");
    }
    call_state(&mut tab).await
}

async fn call_state(tab: &mut Tab) -> Result<WaCall, String> {
    let v = tab.eval(CALL_JS).await?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The call as the watcher last saw it (state "none" when there is none).
static CALL_NOW: std::sync::Mutex<WaCall> =
    std::sync::Mutex::new(WaCall { state: String::new(), name: String::new(), time: String::new(), status: String::new(), mic_off: false, cam_off: false });

/// What the user wants done to the call, from what they said. None = not a call command.
fn call_command(said: &str, state: &str) -> Option<&'static str> {
    let t: String = fold(said).chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect();
    let mut t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    for pre in ["jarvis ", "por favor ", "pode "] {
        if let Some(rest) = t.strip_prefix(pre) {
            t = rest.to_string();
        }
    }
    let about_call = ["chamada", "ligacao", "telefone", "whatsapp"].iter().any(|w| t.contains(w));
    let has = |words: &[&str]| words.iter().any(|w| t.starts_with(w) || t.contains(&format!(" {w}")));
    let in_call = state == "active";
    let ringing = state == "incoming";
    // Mute first: "desliga o microfone" is not hanging up.
    if t.contains("microfone") || t.contains("mudo") || has(&["muta", "mutar", "silencia", "silenciar", "desmuta", "desmutar"]) {
        return (in_call || about_call).then_some("mute");
    }
    if has(&["atende", "atender", "atenda", "aceita", "aceitar"]) && (ringing || about_call) {
        return Some("accept");
    }
    if has(&["recusa", "recusar", "recuse", "rejeita", "rejeitar", "nao atende", "nao atender", "ignora"]) && (ringing || about_call) {
        return Some("decline");
    }
    if has(&["desliga", "desligar", "desligue", "encerra", "encerrar", "encerre", "termina", "terminar", "finaliza", "finalizar", "sai da", "sair da", "corta"]) {
        // "Desliga o PC" / "desliga a luz": not the call.
        if about_call || ((in_call || ringing) && matches!(t.as_str(), "desliga" | "desligar" | "desliga ai" | "encerra" | "pode desligar")) {
            return Some(if ringing { "decline" } else { "hangup" });
        }
    }
    None
}

/// "Desliga a ligação", "atende", "recusa", "muta o microfone" — said to
/// Jarvis about a WhatsApp call: done at once on the call's own buttons.
pub async fn quick_call_control(said: &str) -> Option<String> {
    let now = CALL_NOW.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let action = call_command(said, &now.state)?;
    if now.state.is_empty() || now.state == "none" {
        return Some("Não tem nenhuma ligação do WhatsApp agora.".into());
    }
    Some(match call_action(action).await {
        Ok(after) => match action {
            "hangup" => "Ligação encerrada.".into(),
            "decline" => "Ligação recusada.".into(),
            "accept" => format!("Atendi{}.", if now.name.is_empty() { String::new() } else { format!(" {}", now.name) }),
            _ => if after.mic_off { "Microfone da ligação desligado.".into() } else { "Microfone da ligação ligado.".into() },
        },
        Err(e) => format!("Não consegui: {e}"),
    })
}

/// Watches for calls while WhatsApp is loaded; tells the notch when one
/// starts, rings in, ticks or ends.
fn spawn_call_watch() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        let mut last = WaCall { state: "none".into(), ..Default::default() };
        loop {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            let Some(app) = APP.get() else { continue };
            if app.get_webview_window(LABEL).is_none() {
                continue;
            }
            // Never in the way of a request (it holds the page for a moment).
            let Ok(_busy) = BUSY.try_lock() else { continue };
            let Ok(mut tab) = Tab::get().await else { continue };
            let Ok(now) = call_state(&mut tab).await else { continue };
            drop(_busy);
            *CALL_NOW.lock().unwrap_or_else(|e| e.into_inner()) = now.clone();
            if now != last {
                if now.state != last.state {
                    crate::log::line(format!("whatsapp: chamada {} {}", now.state, now.name));
                }
                let _ = tauri::Emitter::emit_to(app, crate::island::WINDOW_LABEL, "whatsapp-call", &now);
                last = now;
            }
        }
    });
}

// ── Watched contacts: "a Carol te mandou mensagem, quer que eu leia?" ─────────

/// Where the conversation about a watched contact's message stands.
#[derive(Debug, Clone, PartialEq)]
enum Incoming {
    /// "X te mandou mensagem. Quer que eu leia?" (how many new ones)
    AskRead(String, u32),
    /// Read out; "Quer responder?"
    AskReply(String),
    /// "O que você quer responder?"
    Dictate(String),
}

static INCOMING: std::sync::Mutex<Option<(Incoming, Instant)>> = std::sync::Mutex::new(None);

fn set_incoming(step: Option<Incoming>) {
    *INCOMING.lock().unwrap_or_else(|e| e.into_inner()) = step.map(|s| (s, Instant::now()));
}

/// The last message the contact sent in the open chat (its time and text), to
/// notice new ones while that chat is open (they never count as unread).
fn last_incoming_js() -> String {
    format!(
        r#"(() => {{
  const isMine = {MINE_JS};
  const rows = [...document.querySelectorAll('#main [data-pre-plain-text]')].filter(r => !isMine(r));
  const r = rows[rows.length - 1];
  return r ? (r.getAttribute('data-pre-plain-text') || '') + (r.innerText || '').slice(0, 200) : '';
}})()"#
    )
}

/// Every few seconds: a new message from a watched contact → the notch says so
/// and asks before reading (nothing is read on its own).
fn spawn_message_watch() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        // What was last seen per contact; the first look only notes it down.
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut open_seen: Option<(String, String)> = None;
        let lastin_js = last_incoming_js();
        loop {
            tokio::time::sleep(Duration::from_secs(6)).await;
            let Some(app) = APP.get() else { continue };
            if app.get_webview_window(LABEL).is_none() {
                continue;
            }
            let watched: Vec<String> = crate::jarvis_store::load_data().whatsapp_watch.iter().map(|w| resolve_contact(w)).collect();
            if watched.is_empty() {
                continue;
            }
            let Ok(_busy) = BUSY.try_lock() else { continue };
            let Ok(mut tab) = Tab::get().await else { continue };
            let Ok(v) = tab.eval(CHATS_JS).await else { continue };
            let chats: Vec<WaChat> = serde_json::from_value(v).unwrap_or_default();
            let open = match tab.eval(TITLE_JS).await {
                Ok(Value::String(t)) => t,
                _ => String::new(),
            };
            let last_in = if open.is_empty() { String::new() } else { tab.eval(&lastin_js).await.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default() };
            drop(_busy);

            let mut news: Vec<(String, u32)> = Vec::new();
            for w in &watched {
                let key = fold(w);
                if let Some(c) = chats.iter().find(|c| fold(&c.title) == key) {
                    let sig = format!("{}|{}|{}", c.last, c.time, c.unread);
                    let before = seen.insert(key.clone(), sig.clone());
                    if c.unread > 0 && before.is_some_and(|b| b != sig) {
                        news.push((c.title.clone(), c.unread));
                    }
                }
                // That chat is the one open in the page: its messages are read
                // at once, so a new one is told by the last message itself.
                if fold(&open) == key && !last_in.is_empty() {
                    let before = open_seen.replace((key.clone(), last_in.clone()));
                    if before.is_some_and(|(k, b)| k == key && b != last_in) && !news.iter().any(|(t, _)| fold(t) == key) {
                        news.push((open.clone(), 1));
                    }
                }
            }
            for (title, count) in news {
                // Already asked about this one a moment ago: only the count grows.
                let pending = INCOMING.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some((Incoming::AskRead(t, n), at)) = pending {
                    if fold(&t) == fold(&title) && at.elapsed() < Duration::from_secs(60) {
                        *INCOMING.lock().unwrap_or_else(|e| e.into_inner()) = Some((Incoming::AskRead(t, n.max(count)), at));
                        continue;
                    }
                }
                let text = if count > 1 {
                    format!("{title} te mandou {count} mensagens no WhatsApp. Quer que eu leia?")
                } else {
                    format!("{title} te mandou uma mensagem no WhatsApp. Quer que eu leia?")
                };
                crate::log::line(format!("whatsapp: mensagem nova de {title} (avisando)"));
                set_incoming(Some(Incoming::AskRead(title.clone(), count)));
                let _ = tauri::Emitter::emit_to(app, crate::island::WINDOW_LABEL, "whatsapp-incoming", json!({ "contact": title, "text": text }));
            }
        }
    });
}

/// What the contact sent, said in a few words ("mandou um áudio").
fn spoken_message(m: &WaMessage) -> String {
    let text = m.text.trim();
    match m.kind.as_str() {
        "audio" => format!("mandou um áudio{}", if m.duration.is_empty() { String::new() } else { format!(" de {}", m.duration) }),
        "image" if text.is_empty() => "mandou uma foto".into(),
        "image" => format!("mandou uma foto: {text}"),
        "video" => "mandou um vídeo".into(),
        "document" => format!("mandou um documento{}", if text.is_empty() { String::new() } else { format!(": {text}") }),
        "sticker" => "mandou uma figurinha".into(),
        _ if text.is_empty() => "mandou uma mensagem".into(),
        _ => format!("disse: {text}"),
    }
}

/// Reads the contact's newest messages (the `count` last ones they sent).
async fn read_new(contact: &str, count: u32) -> Result<String, String> {
    let t = thread(contact, 30).await?;
    let theirs: Vec<&WaMessage> = t.messages.iter().filter(|m| !m.mine).collect();
    let n = (count.max(1) as usize).min(6).min(theirs.len());
    if n == 0 {
        return Ok(format!("Abri a conversa com {}, mas não achei a mensagem.", t.title));
    }
    let lines: Vec<String> = theirs[theirs.len() - n..].iter().map(|m| spoken_message(m)).collect();
    Ok(format!("{} {}. Quer responder?", t.title, lines.join(". Depois, ")))
}

/// "Responde que já tô chegando" → "já tô chegando".
fn reply_text(said: &str) -> String {
    let mut s = said.trim().to_string();
    for _ in 0..2 {
        let f = fold(&s);
        for pre in [
            "jarvis ", "jarvis, ", "responde que ", "responda que ", "responder que ", "fala que ", "fale que ", "diz que ", "diga que ", "manda que ",
            "mande que ", "escreve que ", "escreva que ", "responde ", "responda ", "manda ", "mande ", "fala ", "diz ",
        ] {
            if f.starts_with(pre) {
                s = s.chars().skip(pre.chars().count()).collect::<String>().trim().to_string();
                break;
            }
        }
    }
    s
}

/// The answers in the watched-contact conversation. None = not part of it.
async fn answer_incoming(said: &str) -> Option<String> {
    let (step, at) = INCOMING.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    if at.elapsed() > HOLD_FOR {
        set_incoming(None);
        return None;
    }
    let kind = answer_kind(said);
    match step {
        Incoming::AskRead(contact, count) => match kind {
            Answer::Yes => Some(match read_new(&contact, count).await {
                Ok(text) => {
                    set_incoming(Some(Incoming::AskReply(contact)));
                    text
                }
                Err(e) => {
                    set_incoming(None);
                    e
                }
            }),
            Answer::No => {
                set_incoming(None);
                Some("Ok, deixo para depois.".into())
            }
            Answer::Other => {
                set_incoming(None);
                None
            }
        },
        Incoming::AskReply(contact) | Incoming::Dictate(contact) if kind == Answer::No => {
            let _ = contact;
            set_incoming(None);
            Some("Ok, sem resposta.".into())
        }
        Incoming::AskReply(contact) if kind == Answer::Yes => {
            set_incoming(Some(Incoming::Dictate(contact)));
            Some("O que você quer responder?".into())
        }
        Incoming::AskReply(contact) | Incoming::Dictate(contact) => {
            let text = reply_text(said);
            if text.is_empty() {
                return Some("O que você quer responder?".into());
            }
            set_incoming(None);
            // The usual check before anything is sent: "Posso enviar?".
            let _ = prepare(&contact, &text);
            Some(format!("Vou mandar para {}: \"{}\". Posso enviar?", who(&contact), text))
        }
    }
}

/// "Avisa quando a Carol mandar mensagem" (or stop): the brain's tool.
pub fn watch(contact: &str, on: bool) -> Result<String, String> {
    let name = resolve_contact(contact);
    crate::jarvis_store::set_watch(&name, on)?;
    Ok(if on {
        format!("Combinado: quando {name} mandar mensagem no WhatsApp, eu aviso e pergunto se posso ler.")
    } else {
        format!("Ok, não aviso mais das mensagens de {name}.")
    })
}

/// Sends `message` to `to`. From the notch (the user typed it) or after
/// Permitir (the brain).
pub async fn send(to: &str, message: &str) -> Result<String, String> {
    let text: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err("A mensagem está vazia.".into());
    }
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    let chat = open_chat(&mut tab, to).await?;
    let compose = tab
        .eval(
            r#"(() => {
  const b = document.querySelector('#main footer [contenteditable="true"]');
  if (!b) return null;
  b.focus();
  const r = b.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})()"#,
        )
        .await?;
    if compose.is_null() {
        return Err("Não achei a caixa de mensagem da conversa.".into());
    }
    tab.click(&compose).await?;
    tab.eval(CLEAR_FOCUSED_JS).await?;
    tab.insert_text(&text).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    tab.enter().await?;

    // Sent = the box emptied and the text is now at the end of the chat. Not
    // tied to WhatsApp's class names (they change); letters and digits only,
    // since emoji render as images.
    let check = format!(
        r#"(() => {{
  const norm = s => (s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '');
  const probe = norm({text}).slice(0, 30);
  const box = document.querySelector('#main footer [contenteditable="true"]');
  if (!box || (box.innerText || '').trim()) return false;
  const tail = norm(document.querySelector('#main')?.innerText || '').slice(-4000);
  return !probe || tail.includes(probe);
}})()"#,
        text = serde_json::to_string(&text).unwrap_or_default()
    );
    let ok = wait_for(&mut tab, &check, 8_000).await.is_some();
    log_structure(&mut tab).await;
    if ok {
        crate::log::line(format!("whatsapp: mensagem enviada para {chat}"));
        Ok(format!("Mensagem enviada no WhatsApp para {chat}."))
    } else {
        Err(format!("Digitei a mensagem para {chat}, mas não consegui confirmar o envio. Confira no WhatsApp."))
    }
}

/// Once per run: how WhatsApp marks the messages today (class names and
/// counts only, never their text), to keep the selectors right.
async fn log_structure(tab: &mut Tab) {
    static DONE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    // A couple of times per run is enough.
    if DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2 {
        return;
    }
    let js = r#"(() => {
  const msgs = document.querySelectorAll('#main [data-pre-plain-text]');
  const last = msgs[msgs.length - 1];
  const chain = [];
  for (let e = last, i = 0; e && i < 7; e = e.parentElement, i++) {
    chain.push((e.className && typeof e.className === 'string' ? e.className.split(/\s+/).filter(c => /message|in|out|focus|row/i.test(c)).slice(0, 3).join('.') : '') || e.tagName.toLowerCase());
  }
  const ids = [...document.querySelectorAll('#main [data-id]')].map(e => e.getAttribute('data-id') || '');
  return { prePlain: msgs.length, out: document.querySelectorAll('#main .message-out').length, in: document.querySelectorAll('#main .message-in').length,
    idTrue: ids.filter(i => i.startsWith('true_')).length, idFalse: ids.filter(i => i.startsWith('false_')).length,
    checkIcons: document.querySelectorAll('#main [data-icon^="msg-check"], #main [data-icon^="msg-dblcheck"]').length,
    roleRows: document.querySelectorAll('#main [role="row"]').length, chain: chain.join(' < '),
    // Which side each of the last messages sits on (no text).
    sides: (() => { const m = document.querySelector('#main').getBoundingClientRect();
      return [...msgs].slice(-6).map(x => { const b = x.getBoundingClientRect(); return b.width ? ((b.left + b.right) / 2 > (m.left + m.right) / 2 ? 'R' : 'L') : '?'; }).join(''); })() };
})()"#;
    if let Ok(v) = tab.eval(js).await {
        crate::log::line(format!("whatsapp: estrutura da conversa {v}"));
    }
}

/// For the brain: the last messages of a chat as text.
pub async fn read(chat: &str, count: usize) -> Result<String, String> {
    let t = thread(chat, count.clamp(1, 40)).await?;
    if t.messages.is_empty() {
        return Ok(format!("Conversa \"{}\" aberta, mas não há mensagens de texto carregadas.", t.title));
    }
    let mut out = format!("Últimas mensagens de \"{}\":\n", t.title);
    for m in t.messages {
        let body = match m.kind.as_str() {
            "audio" => format!("[áudio {}]", m.duration),
            "image" => format!("[imagem]{}", if m.text.is_empty() { String::new() } else { format!(" {}", m.text) }),
            "video" => "[vídeo]".into(),
            "document" => "[documento]".into(),
            "sticker" => "[figurinha]".into(),
            _ => m.text.clone(),
        };
        out.push_str(&format!("[{}] {}: {}\n", m.when, m.from, body));
    }
    Ok(out)
}

/// For the brain: chats with unread messages.
pub async fn unread() -> Result<String, String> {
    let list: Vec<WaChat> = chats().await?.into_iter().filter(|c| c.unread > 0).collect();
    if list.is_empty() {
        return Ok("Nenhuma conversa com mensagens não lidas (entre as mais recentes).".into());
    }
    let mut out = format!("{} conversa(s) com mensagens não lidas:\n", list.len());
    for c in list {
        let last = if c.last.is_empty() { String::new() } else { format!(" — \"{}\"", c.last) };
        out.push_str(&format!("- {} ({}){last}\n", c.title, c.unread));
    }
    Ok(out)
}

// ── Confirming in the conversation ───────────────────────────────────────────
//
// A message the brain wants to send is held here, Jarvis reads back who and
// what and asks "Posso enviar?". The user's next words decide: "sim" sends it,
// "não" drops it (answered in jarvis_ask before the AI), anything else goes to
// the AI as usual (which may prepare a corrected message).

struct Held {
    to: String,
    message: String,
    at: Instant,
}

static HELD: std::sync::Mutex<Option<Held>> = std::sync::Mutex::new(None);
/// After this the question is stale: "sim" no longer sends anything.
const HOLD_FOR: Duration = Duration::from_secs(180);

fn who(to: &str) -> String {
    if is_self(to) {
        "você mesmo".into()
    } else {
        to.trim().to_string()
    }
}

/// "Carol" → "Carolina Braga" when the user gave that nickname (Organizar →
/// Apelidos, or by voice); anything else as it is.
pub fn resolve_contact(name: &str) -> String {
    let n = name.trim();
    let key = fold(n);
    let key = key.trim_start_matches("a ").trim_start_matches("o ").trim();
    let list = crate::jarvis_store::load_data().contact_nicknames;
    // Exact, else one letter off at the end ("Caro" heard for "Carol").
    list.iter()
        .find(|c| fold(&c.nick) == key)
        .or_else(|| {
            list.iter().find(|c| {
                let nk = fold(&c.nick);
                key.chars().count() >= 3 && (nk.starts_with(key) || key.starts_with(nk.as_str())) && nk.chars().count().abs_diff(key.chars().count()) == 1
            })
        })
        .map(|c| c.contact.clone())
        .unwrap_or_else(|| n.to_string())
}

/// Saves a nickname, with the contact's name as WhatsApp shows it when the
/// search finds it.
pub async fn add_nickname(nick: &str, contact: &str) -> Result<String, String> {
    let mut name = contact.trim().to_string();
    let found = tokio::time::timeout(Duration::from_secs(10), search_chats(contact)).await;
    if let Ok(Ok(list)) = found {
        if let Some(c) = list.iter().find(|c| fold(&c.title) == fold(contact)).or(if list.len() == 1 { list.first() } else { None }) {
            name = c.title.clone();
        }
    }
    crate::jarvis_store::set_nickname(nick, &name)?;
    Ok(format!("Pronto: quando você disser \"{}\", vou entender {} no WhatsApp.", nick.trim(), name))
}

/// The brain's `whatsapp_send`. The first call holds the message and returns
/// what Jarvis must ask; the same message again, in a later turn, is the
/// user's yes (when it reached the AI instead of the quick answer below).
pub enum Prepared {
    /// Ask the user (the text tells the AI what to say).
    Ask(String),
    /// Confirmed: send it now.
    Send(String, String),
}

pub fn prepare(to: &str, message: &str) -> Prepared {
    let to = resolve_contact(to);
    let to = to.as_str();
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = held.as_ref() {
        let same = fold(&h.to) == fold(to) && h.message.trim() == message.trim();
        let later_turn = h.at.elapsed() > Duration::from_secs(3) && h.at.elapsed() < HOLD_FOR;
        if same && later_turn {
            let h = held.take().unwrap();
            return Prepared::Send(h.to, h.message);
        }
    }
    *held = Some(Held { to: to.trim().to_string(), message: message.trim().to_string(), at: Instant::now() });
    Prepared::Ask(format!(
        "Mensagem PREPARADA, ainda NÃO enviada. Responda ao usuário exatamente assim e nada mais: Vou mandar para {}: \"{}\". Posso enviar?",
        who(to),
        message.trim()
    ))
}

#[derive(Debug, PartialEq)]
pub enum Answer {
    Yes,
    No,
    /// Not an answer to the question (a correction, another request…).
    Other,
}

/// "sim", "pode mandar", "manda aí" / "não", "cancela", "deixa pra lá".
pub fn answer_kind(said: &str) -> Answer {
    let t: String = fold(said).chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect();
    let mut words: Vec<&str> = t.split_whitespace().collect();
    // "Jarvis, sim" — the name said again out of habit.
    if words.first().is_some_and(|w| w.starts_with("jarv") || *w == "javis") {
        words.remove(0);
    }
    if words.is_empty() || words.len() > 8 {
        return Answer::Other;
    }
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    let changes = ["muda", "mude", "troca", "troque", "altera", "altere", "corrige", "acrescenta", "coloca", "escreve", "vez", "mas"];
    if has(&changes) {
        return Answer::Other;
    }
    let no = ["nao", "cancela", "cancelar", "cancele", "esquece", "deixa", "negativo", "nunca"];
    if has(&no) {
        return Answer::No;
    }
    let yes = [
        "sim", "pode", "manda", "mande", "envia", "envie", "enviar", "mandar", "confirmo", "confirma", "confirmado", "isso", "ok", "okay",
        "claro", "beleza", "positivo", "certo", "perfeito", "bora", "vai", "s", "aham", "uhum", "exato",
    ];
    if has(&yes) {
        Answer::Yes
    } else {
        Answer::Other
    }
}

/// Answers the held question when `said` is a yes or a no. None = nothing
/// held (or not an answer): the request goes to the AI as usual.
/// A call waiting for "Posso ligar?": (who, video, when asked).
static HELD_CALL: std::sync::Mutex<Option<(String, bool, Instant)>> = std::sync::Mutex::new(None);

/// "Liga para a Carolina (no WhatsApp)", "faz uma chamada de vídeo com o Pedro":
/// the person and whether it is video. None = not a call request.
pub fn parse_call(said: &str) -> Option<(String, bool)> {
    let t: String = fold(said).chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect();
    let mut t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    for pre in ["jarvis ", "por favor ", "jarvis por favor "] {
        if let Some(rest) = t.strip_prefix(pre) {
            t = rest.to_string();
        }
    }
    let video = t.contains("video");
    let starts = [
        "me liga para ", "liga para ", "liga pra ", "liga pro ", "ligar para ", "ligar pra ", "ligar pro ", "ligue para ", "ligue pra ",
        "faz uma ligacao para ", "faz uma ligacao pra ", "faca uma ligacao para ", "fazer uma ligacao para ", "faz uma chamada para ",
        "faz uma chamada pra ", "faz uma chamada com ", "faca uma chamada para ", "faz uma chamada de video para ", "faz uma chamada de video pra ",
        "faz uma chamada de video com ", "faca uma chamada de video para ", "chamada de video para ", "chamada de video com ", "ligacao de video para ",
        "liga por video para ", "liga em video para ",
    ];
    let rest = starts.iter().find_map(|p| t.strip_prefix(p))?;
    let mut name = rest.to_string();
    for tail in [" no whatsapp", " pelo whatsapp", " por favor", " agora", " de video", " por video", " em video"] {
        if let Some(i) = name.find(tail) {
            name.truncate(i);
        }
    }
    let name = name.trim().trim_start_matches("a ").trim_start_matches("o ").trim().to_string();
    if name.is_empty() || name.split(' ').count() > 5 {
        return None;
    }
    // Back to how it was said (accents, capitals) for the search.
    let words = name.split(' ').count();
    let original: Vec<&str> = said.split_whitespace().collect();
    let pretty = original
        .windows(words)
        .find(|w| fold(&w.join(" ")).replace(|c: char| !c.is_alphanumeric() && c != ' ', "") == name)
        .map(|w| w.join(" "))
        .unwrap_or(name);
    Some((pretty, video))
}

/// Holds a call and returns what Jarvis asks ("Posso ligar?").
pub fn prepare_call(to: &str, video: bool) -> String {
    let to = resolve_contact(to);
    let to = to.as_str();
    *HELD_CALL.lock().unwrap_or_else(|e| e.into_inner()) = Some((to.trim().to_string(), video, Instant::now()));
    format!("Vou fazer uma {} para {} no WhatsApp. Posso ligar?", if video { "chamada de vídeo" } else { "ligação" }, who(to))
}

/// A call request said straight to Jarvis: asked back at once, no AI needed.
pub fn quick_call(said: &str) -> Option<String> {
    let (to, video) = parse_call(said)?;
    Some(prepare_call(&to, video))
}

pub async fn answer_held(said: &str) -> Option<String> {
    // "X te mandou mensagem, quer que eu leia?" and what follows.
    if let Some(text) = answer_incoming(said).await {
        return Some(text);
    }
    // A call asked about first.
    let call_kind = {
        let held = HELD_CALL.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_ref() {
            Some((_, _, at)) if at.elapsed() < HOLD_FOR => Some(answer_kind(said)),
            _ => None,
        }
    };
    if let Some(kind) = call_kind.filter(|k| *k != Answer::Other) {
        let (to, video, _) = HELD_CALL.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        return Some(match kind {
            Answer::Yes => match call(&to, video).await {
                Ok(msg) => msg,
                Err(e) => e,
            },
            _ => "Ok, não liguei.".into(),
        });
    }
    let kind = {
        let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_ref() {
            Some(h) if h.at.elapsed() < HOLD_FOR => answer_kind(said),
            _ => return None,
        }
    };
    let taken = match kind {
        Answer::Other => return None,
        _ => HELD.lock().unwrap_or_else(|e| e.into_inner()).take()?,
    };
    Some(match kind {
        Answer::Yes => match send(&taken.to, &taken.message).await {
            Ok(_) => format!("Pronto, mandei para {}.", who(&taken.to)),
            Err(e) => e,
        },
        _ => "Ok, não enviei.".into(),
    })
}

// ── Commands for the notch tab ───────────────────────────────────────────────

#[tauri::command]
pub async fn whatsapp_state() -> Result<WaState, String> {
    state().await
}

#[tauri::command]
pub async fn whatsapp_chats(limit: Option<usize>, filter: Option<String>) -> Result<WaChatPage, String> {
    chat_page_filtered(limit.unwrap_or(30), filter.as_deref().unwrap_or("all")).await
}

#[tauri::command]
pub async fn whatsapp_search(query: String) -> Result<Vec<WaChat>, String> {
    search_chats(&query).await
}

#[tauri::command]
pub async fn whatsapp_play(chat: String, id: String) -> Result<String, String> {
    play_audio(&chat, &id).await
}

#[tauri::command]
pub async fn whatsapp_image(chat: String, id: String) -> Result<String, String> {
    image(&chat, &id).await
}

#[tauri::command]
pub fn whatsapp_save_image(data_url: String, name: String) -> Result<String, String> {
    save_image(&data_url, &name)
}

#[tauri::command]
pub async fn whatsapp_voice(chat: String, action: String) -> Result<String, String> {
    voice_note(&chat, &action).await
}

#[tauri::command]
pub async fn whatsapp_thread(chat: String, count: Option<usize>) -> Result<WaThread, String> {
    thread(&chat, count.unwrap_or(30)).await
}

#[tauri::command]
pub async fn whatsapp_call(chat: String, video: bool) -> Result<String, String> {
    call(&chat, video).await
}

#[tauri::command]
pub async fn whatsapp_call_action(action: String) -> Result<WaCall, String> {
    call_action(&action).await
}

/// The user typed it in the notch: sent right away.
#[tauri::command]
pub async fn whatsapp_send(chat: String, text: String) -> Result<String, String> {
    send(&chat, &text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_themself_by_many_names() {
        for s in ["eu", "Mim", "pra mim", "para mim", "Eu mesmo", "meu número", "comigo"] {
            assert!(is_self(s), "{s}");
        }
        for s in ["Ana", "Mãe", "Eugênio", "Daniel Levy"] {
            assert!(!is_self(s), "{s}");
        }
    }

    #[test]
    fn yes_and_no_to_send_or_not() {
        for s in ["Sim", "sim, pode mandar", "Pode enviar.", "manda aí", "Jarvis, sim", "isso mesmo", "ok", "confirmo", "sim, manda para ele"] {
            assert_eq!(answer_kind(s), Answer::Yes, "{s}");
        }
        for s in ["Não", "não manda não", "cancela", "deixa pra lá", "esquece"] {
            assert_eq!(answer_kind(s), Answer::No, "{s}");
        }
        for s in ["muda para oi tudo bem", "sim, mas troca o texto", "que horas são em Tóquio agora meu amigo querido", "toca uma música"] {
            assert_eq!(answer_kind(s), Answer::Other, "{s}");
        }
    }

    /// The tests that use the held message take turns.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_watched_contact_conversation_goes_read_reply_confirm() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(reply_text("Responde que já tô chegando"), "já tô chegando");
        assert_eq!(reply_text("Jarvis, diz que te ligo depois"), "te ligo depois");
        assert_eq!(reply_text("Beleza, às 8"), "Beleza, às 8");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            // "Quer que eu leia?" → "não".
            set_incoming(Some(Incoming::AskRead("Ana Teste".into(), 1)));
            assert_eq!(answer_incoming("não").await.as_deref(), Some("Ok, deixo para depois."));
            assert!(INCOMING.lock().unwrap().is_none());
            // After reading: "Quer responder?" → "sim" → what to say → the usual "Posso enviar?".
            set_incoming(Some(Incoming::AskReply("Ana Teste".into())));
            assert_eq!(answer_incoming("sim").await.as_deref(), Some("O que você quer responder?"));
            let said = answer_incoming("Diz que te ligo depois").await.unwrap();
            assert_eq!(said, "Vou mandar para Ana Teste: \"te ligo depois\". Posso enviar?");
            assert!(HELD.lock().unwrap().as_ref().is_some_and(|h| h.to == "Ana Teste" && h.message == "te ligo depois"));
            *HELD.lock().unwrap() = None;
            // Straight away: "Quer responder?" → "responde que chego às 8".
            set_incoming(Some(Incoming::AskReply("Ana Teste".into())));
            assert!(answer_incoming("responde que chego às 8").await.unwrap().contains("\"chego às 8\""));
            *HELD.lock().unwrap() = None;
            // Something else entirely while asked to read: not this conversation.
            set_incoming(Some(Incoming::AskRead("Ana Teste".into(), 1)));
            assert_eq!(answer_incoming("que horas são em Tóquio agora meu amigo querido").await, None);
            assert!(answer_incoming("sim").await.is_none());
        });
    }

    #[test]
    fn the_same_message_again_later_is_the_yes() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *HELD.lock().unwrap() = None;
        assert!(matches!(prepare("Ana", "oi"), Prepared::Ask(_)));
        // Same turn (a double call): still only asking.
        assert!(matches!(prepare("Ana", "oi"), Prepared::Ask(_)));
        // Pretend the question was asked a while ago.
        HELD.lock().unwrap().as_mut().unwrap().at = Instant::now() - Duration::from_secs(10);
        assert!(matches!(prepare("ana", "oi"), Prepared::Send(..)));
        assert!(HELD.lock().unwrap().is_none());
    }

    #[test]
    fn call_commands_are_understood() {
        assert_eq!(call_command("Desliga a chamada.", "active"), Some("hangup"));
        assert_eq!(call_command("Jarvis, encerra a ligação", "active"), Some("hangup"));
        assert_eq!(call_command("pode desligar", "active"), Some("hangup"));
        assert_eq!(call_command("termina a ligação do WhatsApp", "none"), Some("hangup"));
        assert_eq!(call_command("desliga o microfone", "active"), Some("mute"));
        assert_eq!(call_command("atende", "incoming"), Some("accept"));
        assert_eq!(call_command("recusa a ligação", "incoming"), Some("decline"));
        assert_eq!(call_command("desliga", "incoming"), Some("decline"));
        // Not about a call.
        assert_eq!(call_command("Desligar PC.", "active"), None);
        assert_eq!(call_command("desliga a luz da sala", "none"), None);
        assert_eq!(call_command("desliga", "none"), None);
        assert_eq!(call_command("desliga o microfone", "none"), None);
        assert_eq!(call_command("toca uma música", "active"), None);
    }

    #[test]
    fn call_requests_are_understood() {
        assert_eq!(parse_call("Ligar para Carolina Braga no WhatsApp."), Some(("Carolina Braga".into(), false)));
        assert_eq!(parse_call("Jarvis, liga pra Ana"), Some(("Ana".into(), false)));
        assert_eq!(parse_call("faz uma chamada de vídeo com o Pedro"), Some(("Pedro".into(), true)));
        assert_eq!(parse_call("Liga para a mãe por favor"), Some(("mãe".into(), false)));
        assert_eq!(parse_call("toca uma música"), None);
        assert_eq!(parse_call("liga a luz da sala"), None);
    }

    #[test]
    fn numbers_open_directly_names_do_not() {
        assert_eq!(phone_digits("+55 (71) 99999-0000").as_deref(), Some("5571999990000"));
        assert_eq!(phone_digits("71 99999 0000").as_deref(), Some("71999990000"));
        assert_eq!(phone_digits("Ana 2"), None);
        assert_eq!(phone_digits("12345"), None);
    }
}

#[cfg(test)]
mod nick_probe {
    /// Writes and removes a test nickname in this PC's data.
    /// `cargo test --lib whatsapp::nick_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_nickname_becomes_the_contact() {
        crate::jarvis_store::set_nickname("Zezinho Teste", "José Teste da Silva").unwrap();
        println!("{:?}", super::resolve_contact("zezinho teste"));
        println!("{:?}", super::prepare_call("o Zezinho Teste", false));
        *super::HELD_CALL.lock().unwrap() = None;
        println!("removido: {:?}", crate::jarvis_store::remove_nickname("Zezinho Teste"));
        println!("{:?}", super::resolve_contact("Zezinho Teste"));
    }
}
