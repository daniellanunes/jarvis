//! The Jarvis browser: Chrome (or Edge) with a profile of its own, driven over
//! the Chrome DevTools Protocol on 127.0.0.1. Gives the brain real eyes and
//! hands on web pages: a numbered snapshot of what can be clicked or typed
//! into, trusted mouse/keyboard events, and a fresh snapshot after every
//! action so it can check what really happened instead of guessing.
//!
//! Chrome refuses remote debugging on the user's everyday profile, so this one
//! lives in `<local_dir>/browser-profile`; the user signs in to sites there once.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const PORT: u16 = 9333;
const SNAPSHOT_LIMIT: usize = 90;

static ACTIVE_TARGET: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())
}

fn base() -> String {
    format!("http://127.0.0.1:{PORT}")
}

fn browser_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(var) {
            let root = PathBuf::from(root);
            candidates.push(root.join(r"Google\Chrome\Application\chrome.exe"));
        }
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(var) {
            candidates.push(PathBuf::from(root).join(r"Microsoft\Edge\Application\msedge.exe"));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

pub fn profile_dir() -> PathBuf {
    crate::platform::local_dir().join("browser-profile")
}

async fn is_up() -> bool {
    match http() {
        Ok(c) => c
            .get(format!("{}/json/version", base()))
            .timeout(Duration::from_millis(600))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Starts the Jarvis browser when it is not running (waits up to ~8 s).
async fn ensure_browser(start_url: Option<&str>) -> Result<bool, String> {
    if is_up().await {
        return Ok(false);
    }
    launch(start_url, false)?;
    wait_up().await?;
    Ok(true)
}

/// Starts the Jarvis browser; `minimized` for work in the background
/// (WhatsApp), else maximized for pages the user watches.
fn launch(start_url: Option<&str>, minimized: bool) -> Result<(), String> {
    let exe = browser_exe().ok_or("Não encontrei o Google Chrome nem o Microsoft Edge instalados.")?;
    let profile = profile_dir();
    let _ = std::fs::create_dir_all(&profile);
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(format!("--remote-debugging-port={PORT}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check"])
        .arg(if minimized { "--start-minimized" } else { "--start-maximized" });
    cmd.arg(start_url.unwrap_or("about:blank"));
    cmd.spawn().map_err(|e| format!("Não consegui abrir o navegador do Jarvis: {e}"))?;
    Ok(())
}

async fn wait_up() -> Result<(), String> {
    for _ in 0..32 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if is_up().await {
            *ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Ok(());
        }
    }
    Err("O navegador do Jarvis não respondeu a tempo.".into())
}

async fn targets() -> Result<Vec<Value>, String> {
    let list: Value = http()?
        .get(format!("{}/json/list", base()))
        .send()
        .await
        .map_err(|e| format!("Navegador do Jarvis indisponível: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(list
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t["type"] == "page" && !t["url"].as_str().unwrap_or("").starts_with("devtools://"))
        .collect())
}

/// The tab Jarvis works in: the one it last used, else the first page.
async fn active_ws() -> Result<String, String> {
    let list = targets().await?;
    let remembered = ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let chosen = remembered
        .and_then(|id| list.iter().find(|t| t["id"] == id.as_str()).cloned())
        // Never browse in the WhatsApp tab (whatsapp.rs keeps it for itself).
        .or_else(|| list.iter().find(|t| !t["url"].as_str().unwrap_or("").starts_with(crate::whatsapp::URL)).cloned());
    let t = match chosen {
        Some(t) => t,
        None => new_tab("about:blank").await?,
    };
    *ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner()) = t["id"].as_str().map(str::to_string);
    t["webSocketDebuggerUrl"].as_str().map(str::to_string).ok_or("Aba sem depuração disponível.".into())
}

async fn new_tab(url: &str) -> Result<Value, String> {
    let mut u = reqwest::Url::parse(&format!("{}/json/new", base())).map_err(|e| e.to_string())?;
    u.set_query(Some(url));
    let t: Value = http()?.put(u).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    *ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner()) = t["id"].as_str().map(str::to_string);
    Ok(t)
}

struct Cdp {
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    id: u64,
}

impl Cdp {
    async fn open() -> Result<Self, String> {
        let url = active_ws().await?;
        Self::connect(&url).await
    }

    async fn connect(url: &str) -> Result<Self, String> {
        let (ws, _) = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(url))
            .await
            .map_err(|_| "Navegador do Jarvis não respondeu.".to_string())?
            .map_err(|e| format!("Falha ao conectar no navegador: {e}"))?;
        Ok(Self { ws, id: 0 })
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.ws
            .send(Message::Text(json!({ "id": id, "method": method, "params": params }).to_string()))
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(20), self.ws.next())
                .await
                .map_err(|_| format!("{method}: o navegador demorou demais"))?
                .ok_or("Conexão com o navegador fechada")?
                .map_err(|e| e.to_string())?;
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["id"].as_u64() == Some(id) {
                if let Some(err) = v.get("error") {
                    return Err(format!("{method}: {}", err["message"].as_str().unwrap_or("erro")));
                }
                return Ok(v["result"].clone());
            }
        }
    }

    async fn eval(&mut self, expr: &str) -> Result<Value, String> {
        let r = self
            .send(
                "Runtime.evaluate",
                json!({ "expression": expr, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("erro de script");
            return Err(format!("Erro na página: {msg}"));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Waits for the page to settle after an action (load + a short quiet period).
    async fn settle(&mut self, max_ms: u64) {
        let start = std::time::Instant::now();
        tokio::time::sleep(Duration::from_millis(250)).await;
        while start.elapsed() < Duration::from_millis(max_ms) {
            if let Ok(v) = self.eval("document.readyState").await {
                if v == "complete" {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    async fn click_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.send("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y })).await?;
        for kind in ["mousePressed", "mouseReleased"] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
        }
        Ok(())
    }

    async fn key(&mut self, key: &str) -> Result<(), String> {
        let (code, vk, text) = match key.to_lowercase().as_str() {
            "enter" => ("Enter", 13, "\r"),
            "tab" => ("Tab", 9, ""),
            "escape" | "esc" => ("Escape", 27, ""),
            "backspace" => ("Backspace", 8, ""),
            "arrowdown" | "down" => ("ArrowDown", 40, ""),
            "arrowup" | "up" => ("ArrowUp", 38, ""),
            "arrowleft" | "left" => ("ArrowLeft", 37, ""),
            "arrowright" | "right" => ("ArrowRight", 39, ""),
            "pagedown" => ("PageDown", 34, ""),
            "pageup" => ("PageUp", 33, ""),
            "space" => (" ", 32, " "),
            other => return Err(format!("Tecla não suportada: {other}")),
        };
        let mut down = json!({ "type": "keyDown", "key": code, "code": code, "windowsVirtualKeyCode": vk });
        if !text.is_empty() {
            down["text"] = json!(text);
        }
        self.send("Input.dispatchKeyEvent", down).await?;
        self.send("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": code, "code": code, "windowsVirtualKeyCode": vk }))
            .await?;
        Ok(())
    }
}

/// Numbers every visible interactive element (data-jv) and lists them, in
/// view first. The ids stay valid until the next snapshot.
const SNAPSHOT_JS: &str = r#"(() => {
  const LIMIT = __LIMIT__;
  const sel = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox],[role=radio],[role=combobox],[role=switch],[role=textbox],[role=searchbox],[role=listbox],[contenteditable=""],[contenteditable=true],[onclick]';
  document.querySelectorAll('[data-jv]').forEach(e => e.removeAttribute('data-jv'));
  const vw = innerWidth, vh = innerHeight, seen = new Set(), cands = [];
  const clean = s => (s || '').replace(/\s+/g, ' ').trim();
  for (const e of document.querySelectorAll(sel)) {
    if (seen.has(e)) continue; seen.add(e);
    const r = e.getBoundingClientRect();
    if (r.width < 3 || r.height < 3) continue;
    const st = getComputedStyle(e);
    if (st.visibility === 'hidden' || st.display === 'none' || +st.opacity === 0) continue;
    const inView = r.bottom > 0 && r.top < vh && r.right > 0 && r.left < vw;
    cands.push({ e, inView, top: r.top });
  }
  cands.sort((a, b) => (b.inView - a.inView) || (a.top - b.top));
  const lines = [];
  let n = 0;
  for (const { e, inView } of cands.slice(0, LIMIT)) {
    n++; e.setAttribute('data-jv', String(n));
    const tag = e.tagName.toLowerCase();
    const role = e.getAttribute('role') || (tag === 'input' ? 'input:' + (e.type || 'text') : tag);
    let label = clean(e.getAttribute('aria-label') || e.innerText || e.getAttribute('title') || e.getAttribute('alt') || e.placeholder || e.getAttribute('name') || e.id);
    if (!label && tag === 'a') label = clean(e.getAttribute('href'));
    const bits = [];
    if ('value' in e && e.value && tag !== 'button' && tag !== 'select') bits.push('valor="' + clean(String(e.value)).slice(0, 60) + '"');
    if (tag === 'select') bits.push('selecionado="' + clean(e.options[e.selectedIndex]?.text || '') + '"', 'opções=' + JSON.stringify([...e.options].slice(0, 15).map(o => clean(o.text))));
    if (e.placeholder) bits.push('placeholder="' + clean(e.placeholder).slice(0, 40) + '"');
    if (e.checked || e.getAttribute('aria-checked') === 'true') bits.push('marcado');
    if (e.getAttribute('aria-selected') === 'true') bits.push('selecionado');
    if (e.getAttribute('aria-expanded') === 'true') bits.push('aberto');
    if (e.disabled || e.getAttribute('aria-disabled') === 'true') bits.push('desativado');
    if (!inView) bits.push('fora da tela');
    lines.push('[' + n + '] ' + role + ' "' + label.slice(0, 80) + '"' + (bits.length ? ' (' + bits.join(', ') + ')' : ''));
  }
  const active = document.activeElement && document.activeElement !== document.body ? (document.activeElement.getAttribute('data-jv') || document.activeElement.tagName.toLowerCase()) : '';
  const text = clean((document.querySelector('main') || document.body)?.innerText || '').slice(0, 1400);
  return { title: document.title, url: location.href, total: cands.length, lines, active, text };
})()"#;

async fn snapshot_with(cdp: &mut Cdp) -> Result<String, String> {
    let v = cdp.eval(&SNAPSHOT_JS.replace("__LIMIT__", &SNAPSHOT_LIMIT.to_string())).await?;
    let lines: Vec<String> = v["lines"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let total = v["total"].as_u64().unwrap_or(0) as usize;
    let mut out = format!(
        "Página: {}\nURL: {}\n",
        v["title"].as_str().unwrap_or(""),
        v["url"].as_str().unwrap_or("")
    );
    if let Some(a) = v["active"].as_str().filter(|s| !s.is_empty()) {
        out.push_str(&format!("Foco: {a}\n"));
    }
    out.push_str("Elementos (use o número):\n");
    out.push_str(&lines.join("\n"));
    if total > lines.len() {
        out.push_str(&format!("\n… +{} elementos (role a página para ver mais)", total - lines.len()));
    }
    out.push_str("\nTexto visível: ");
    out.push_str(v["text"].as_str().unwrap_or(""));
    Ok(out)
}

fn normalize_url(url: &str) -> String {
    let u = url.trim();
    if u.contains("://") || u.starts_with("about:") {
        u.to_string()
    } else if u.contains('.') && !u.contains(' ') {
        format!("https://{u}")
    } else {
        let mut s = reqwest::Url::parse("https://www.google.com/search").unwrap();
        s.query_pairs_mut().append_pair("q", u);
        s.to_string()
    }
}

pub async fn open(url: &str, new_tab_flag: bool) -> Result<String, String> {
    let target = normalize_url(url);
    let started = ensure_browser(Some(&target)).await?;
    if new_tab_flag && !started {
        new_tab(&target).await?;
    }
    let mut cdp = Cdp::open().await?;
    let _ = cdp.send("Page.bringToFront", json!({})).await;
    if !started && !new_tab_flag {
        cdp.send("Page.navigate", json!({ "url": target })).await?;
    }
    cdp.settle(8000).await;
    snapshot_with(&mut cdp).await
}

pub async fn snapshot() -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    snapshot_with(&mut cdp).await
}

/// Finds an element by snapshot number, or by visible text when no number is given.
fn locate_js(id: Option<u64>, text: Option<&str>) -> String {
    let text_js = serde_json::to_string(text.unwrap_or("")).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(() => {{
  const norm = s => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  let e = {id} ? document.querySelector('[data-jv="{id}"]') : null;
  const want = norm({text_js});
  if (!e && want) {{
    const all = [...document.querySelectorAll('a,button,input,select,textarea,summary,label,[role],[onclick],li,span,div')];
    const vis = all.filter(x => {{ const r = x.getBoundingClientRect(); return r.width > 2 && r.height > 2; }});
    e = vis.find(x => norm(x.getAttribute('aria-label') || x.innerText || x.value) === want)
      || vis.filter(x => norm(x.getAttribute('aria-label') || x.innerText || x.value).includes(want))
            .sort((a, b) => norm(a.innerText).length - norm(b.innerText).length)[0];
  }}
  if (!e) return null;
  if (!e.getAttribute('data-jv')) e.setAttribute('data-jv', 't' + Date.now());
  e.scrollIntoView({{ block: 'center', inline: 'center' }});
  const r = e.getBoundingClientRect();
  const x = r.left + r.width / 2, y = r.top + r.height / 2;
  const top = document.elementFromPoint(x, y);
  const covered = top && top !== e && !e.contains(top) && !top.contains(e);
  return {{ x, y, tag: e.tagName.toLowerCase(), label: norm(e.getAttribute('aria-label') || e.innerText || e.value || e.placeholder).slice(0, 60),
           covered: covered ? norm(top.innerText || top.tagName).slice(0, 40) : '' , jv: e.getAttribute('data-jv') || '' }};
}})()"#,
        id = id.unwrap_or(0)
    )
}

async fn locate(cdp: &mut Cdp, id: Option<u64>, text: Option<&str>) -> Result<Value, String> {
    if id.is_none() && text.map_or(true, |t| t.trim().is_empty()) {
        return Err("Informe o número do elemento (do snapshot) ou o texto dele.".into());
    }
    let v = cdp.eval(&locate_js(id, text)).await?;
    if v.is_null() {
        return Err(match id {
            Some(n) => format!("O elemento [{n}] não existe mais na página. Peça um novo snapshot e use o número atualizado."),
            None => format!("Não encontrei nenhum elemento com o texto \"{}\".", text.unwrap_or("")),
        });
    }
    Ok(v)
}

pub async fn click(id: Option<u64>, text: Option<&str>) -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    let before = cdp.eval("location.href").await.unwrap_or(Value::Null);
    let el = locate(&mut cdp, id, text).await?;
    let (x, y) = (el["x"].as_f64().unwrap_or(0.0), el["y"].as_f64().unwrap_or(0.0));
    tokio::time::sleep(Duration::from_millis(120)).await; // let the scroll finish
    cdp.click_at(x, y).await?;
    cdp.settle(5000).await;
    let after = cdp.eval("location.href").await.unwrap_or(Value::Null);
    let mut out = format!("Cliquei em {} \"{}\".", el["tag"].as_str().unwrap_or(""), el["label"].as_str().unwrap_or(""));
    if let Some(c) = el["covered"].as_str().filter(|c| !c.is_empty()) {
        out.push_str(&format!(" Atenção: havia algo por cima (\"{c}\"); confira se o clique teve efeito."));
    }
    if before != after {
        out.push_str(&format!(" A página mudou para {}.", after.as_str().unwrap_or("")));
    }
    out.push_str("\n\nEstado atual (confira se deu certo):\n");
    out.push_str(&snapshot_with(&mut cdp).await?);
    Ok(out)
}

pub async fn type_text(id: Option<u64>, label: Option<&str>, text: &str, submit: bool, clear: bool) -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    let el = locate(&mut cdp, id, label).await?;
    cdp.click_at(el["x"].as_f64().unwrap_or(0.0), el["y"].as_f64().unwrap_or(0.0)).await?;
    if clear {
        // Select everything in the field so the typed text replaces it.
        cdp.eval(
            "(() => { const e = document.activeElement; if (!e) return; if (e.select) e.select(); \
             else if (e.isContentEditable) { const r = document.createRange(); r.selectNodeContents(e); \
             const s = getSelection(); s.removeAllRanges(); s.addRange(r); } })()",
        )
        .await?;
    }
    cdp.send("Input.insertText", json!({ "text": text })).await?;
    let value = cdp
        .eval("(() => { const e = document.activeElement; return e ? (e.value ?? e.innerText ?? '') : ''; })()")
        .await
        .unwrap_or(Value::Null);
    if submit {
        cdp.key("Enter").await?;
        cdp.settle(6000).await;
    }
    let mut out = format!("Digitei \"{text}\". Valor do campo agora: \"{}\".", value.as_str().unwrap_or(""));
    if submit {
        out.push_str(" Enviei com Enter.");
    }
    out.push_str("\n\nEstado atual:\n");
    out.push_str(&snapshot_with(&mut cdp).await?);
    Ok(out)
}

pub async fn select_option(id: Option<u64>, label: Option<&str>, option: &str) -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    let el = locate(&mut cdp, id, label).await?;
    let jv = serde_json::to_string(el["jv"].as_str().unwrap_or("")).unwrap_or_default();
    let opt = serde_json::to_string(option).unwrap_or_default();
    let js = format!(
        r#"(() => {{
  const norm = s => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const e = document.querySelector('[data-jv="' + {jv} + '"]');
  if (!e || e.tagName !== 'SELECT') return {{ ok: false, why: 'not-select' }};
  const want = norm({opt});
  const o = [...e.options].find(o => norm(o.text) === want || norm(o.value) === want)
         || [...e.options].find(o => norm(o.text).includes(want));
  if (!o) return {{ ok: false, why: 'no-option', options: [...e.options].map(o => o.text.trim()).slice(0, 30) }};
  const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
  setter.call(e, o.value);
  e.dispatchEvent(new Event('input', {{ bubbles: true }}));
  e.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return {{ ok: e.value === o.value, chosen: o.text.trim() }};
}})()"#
    );
    let r = cdp.eval(&js).await?;
    if r["why"] == "not-select" {
        return Err("Esse elemento não é uma lista <select>. Para menus personalizados, clique nele (browser_click) e depois clique na opção no novo snapshot.".into());
    }
    if r["why"] == "no-option" {
        return Err(format!("Opção \"{option}\" não existe. Opções: {}", r["options"]));
    }
    cdp.settle(3000).await;
    let mut out = format!("Selecionei \"{}\".", r["chosen"].as_str().unwrap_or(option));
    out.push_str("\n\nEstado atual:\n");
    out.push_str(&snapshot_with(&mut cdp).await?);
    Ok(out)
}

pub async fn press(key: &str) -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    cdp.key(key).await?;
    cdp.settle(4000).await;
    Ok(format!("Tecla {key} pressionada.\n\nEstado atual:\n{}", snapshot_with(&mut cdp).await?))
}

pub async fn scroll(direction: &str) -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    let js = match direction {
        "up" | "cima" => "scrollBy(0, -innerHeight * 0.8)",
        "top" | "topo" => "scrollTo(0, 0)",
        "bottom" | "fim" => "scrollTo(0, document.body.scrollHeight)",
        _ => "scrollBy(0, innerHeight * 0.8)",
    };
    cdp.eval(js).await?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    Ok(format!("Rolei a página.\n\n{}", snapshot_with(&mut cdp).await?))
}

pub async fn back() -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    cdp.eval("history.back()").await?;
    cdp.settle(5000).await;
    snapshot_with(&mut cdp).await
}

pub async fn read_page() -> Result<String, String> {
    ensure_browser(None).await?;
    let mut cdp = Cdp::open().await?;
    let v = cdp
        .eval("({ title: document.title, url: location.href, text: (document.querySelector('main') || document.body).innerText.replace(/\\n{3,}/g, '\\n\\n').slice(0, 7000) })")
        .await?;
    Ok(format!(
        "Página: {}\nURL: {}\n\n{}",
        v["title"].as_str().unwrap_or(""),
        v["url"].as_str().unwrap_or(""),
        v["text"].as_str().unwrap_or("")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_searches_are_told_apart() {
        assert_eq!(normalize_url("youtube.com"), "https://youtube.com");
        assert_eq!(normalize_url("https://x.com/a"), "https://x.com/a");
        assert!(normalize_url("melhor pizza de sp").starts_with("https://www.google.com/search?q=melhor+pizza"));
    }

    #[test]
    fn locate_script_embeds_text_safely() {
        let js = locate_js(None, Some("Entrar \"já\""));
        assert!(js.contains(r#""Entrar \"já\"""#));
        assert!(locate_js(Some(7), None).contains("data-jv=\"7\""));
    }
}
