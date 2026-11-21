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
        let r = self.cdp("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "userGesture": true })).await?;
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

/// Called once at startup. If WhatsApp was linked before, it loads in the
/// background a little later, so the first request is quick.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    if linked_marker().exists() {
        tauri::async_runtime::spawn(async {
            tokio::time::sleep(Duration::from_secs(25)).await;
            let _ = Tab::get().await;
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
  const t = h.querySelector('span[title]') || h.querySelector('[dir="auto"]');
  return (t?.getAttribute('title') || t?.innerText || 'conversa').trim();
})()"#;

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

    // By name: the search box of the chat list.
    let at = wait_for(tab, SEARCH_BOX_JS, 5_000).await.ok_or("Não achei a busca de conversas do WhatsApp.")?;
    tab.click(&at).await?;
    tab.eval(CLEAR_FOCUSED_JS).await?;
    tab.insert_text(to.trim()).await?;
    tokio::time::sleep(Duration::from_millis(1400)).await;

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
    let found = tab.eval(&pick_js).await?;
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
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WaMessage {
    when: String,
    from: String,
    text: String,
    mine: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaThread {
    title: String,
    messages: Vec<WaMessage>,
}

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
    out.push({ title, last, time, unread, top: rect.top });
  }
  return out.sort((a, b) => a.top - b.top).map(({ top, ...c }) => c);
})()"#;

fn messages_js(n: usize) -> String {
    format!(
        r#"(() => {{
  const els = [...document.querySelectorAll('#main [data-pre-plain-text]')].slice(-{n});
  return els.map(e => {{
    const pre = e.getAttribute('data-pre-plain-text') || '';
    const m = pre.match(/^\[(.*?)\]\s*(.*?):\s*$/);
    const mine = !!e.closest('.message-out');
    const text = (e.querySelector('.selectable-text') || e).innerText.trim();
    return {{ when: m ? m[1] : '', from: mine ? 'Você' : (m ? m[2] : ''), text, mine }};
  }});
}})()"#,
        n = n.clamp(1, 60)
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

/// Opens a chat and returns its last `count` messages, oldest first.
pub async fn thread(chat: &str, count: usize) -> Result<WaThread, String> {
    let _busy = BUSY.lock().await;
    let mut tab = ready_tab().await?;
    let title = open_chat(&mut tab, chat).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let v = tab.eval(&messages_js(count)).await?;
    Ok(WaThread { title, messages: serde_json::from_value(v).unwrap_or_default() })
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

    // Confirm it is in the chat as an outgoing message.
    let probe: String = text.chars().take(40).collect();
    let check = format!(
        r#"(() => {{
  const outs = [...document.querySelectorAll('#main .message-out')];
  const last = outs[outs.length - 1];
  const t = last ? (last.querySelector('[data-pre-plain-text]')?.innerText || last.innerText || '') : '';
  return t.replace(/\s+/g, ' ').includes({probe});
}})()"#,
        probe = serde_json::to_string(&probe).unwrap_or_default()
    );
    if wait_for(&mut tab, &check, 6_000).await.is_some() {
        crate::log::line(format!("whatsapp: mensagem enviada para {chat}"));
        Ok(format!("Mensagem enviada no WhatsApp para {chat}."))
    } else {
        Err(format!("Digitei a mensagem para {chat}, mas não consegui confirmar o envio. Confira no WhatsApp."))
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
        out.push_str(&format!("[{}] {}: {}\n", m.when, m.from, m.text));
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

/// What a tool call sends to the brain, as JSON (for jarvis_tools).
pub fn pending_command(to: &str, message: &str) -> String {
    json!({ "to": to, "message": message }).to_string()
}

// ── Commands for the notch tab ───────────────────────────────────────────────

#[tauri::command]
pub async fn whatsapp_state() -> Result<WaState, String> {
    state().await
}

#[tauri::command]
pub async fn whatsapp_chats() -> Result<Vec<WaChat>, String> {
    chats().await
}

#[tauri::command]
pub async fn whatsapp_thread(chat: String) -> Result<WaThread, String> {
    thread(&chat, 30).await
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
    fn numbers_open_directly_names_do_not() {
        assert_eq!(phone_digits("+55 (71) 99999-0000").as_deref(), Some("5571999990000"));
        assert_eq!(phone_digits("71 99999 0000").as_deref(), Some("71999990000"));
        assert_eq!(phone_digits("Ana 2"), None);
        assert_eq!(phone_digits("12345"), None);
    }
}
