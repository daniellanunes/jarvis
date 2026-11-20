//! Multi-account manager & browser-based login flow for Jarvis AI providers
//! (Claude, Gemini, OpenAI/ChatGPT, Groq).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::platform::config_dir;
use crate::secrets;

static ACCOUNT_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAccount {
    pub id: String,
    pub provider: String,    // "claude" | "gemini" | "openai" | "groq"
    pub label: String,       // User-friendly name (e.g. "Pessoal", "Trabalho")
    pub auth_method: String, // "oauth" | "token" | "api_key"
    pub email: Option<String>,
    pub active: bool,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountsStore {
    accounts: Vec<AiAccount>,
}

fn accounts_file_path() -> PathBuf {
    config_dir().join("jarvis_accounts.json")
}

fn load_store() -> AccountsStore {
    let path = accounts_file_path();
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(parsed) = serde_json::from_str::<AccountsStore>(&raw) {
            return parsed;
        }
    }
    AccountsStore::default()
}

fn save_store(store: &AccountsStore) -> Result<(), String> {
    let path = accounts_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Erro ao criar pasta: {e}"))?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Erro ao salvar contas: {e}"))
}

pub fn normalize_provider(provider: &str) -> Result<String, String> {
    match provider.trim().to_lowercase().as_str() {
        "claude" | "anthropic" | "claude-oauth" => Ok("claude".into()),
        "gemini" | "google" | "gemini-oauth" => Ok("gemini".into()),
        "openai" | "chatgpt" => Ok("openai".into()),
        "groq" => Ok("groq".into()),
        other => Err(format!("Provedor não suportado: {other}")),
    }
}

// Per-account tokens live in the keychain under `jarvis-acc-<id>`; secrets.rs
// also reads (and migrates) the ones saved before the app was renamed.

fn set_account_secret(account_id: &str, token: &str) -> Result<(), String> {
    let user = format!("jarvis-acc-{account_id}");
    if token.trim().is_empty() {
        secrets::delete_raw(&user);
        return Ok(());
    }
    secrets::set_raw(&user, token.trim())
}

fn get_account_secret(account_id: &str) -> Option<String> {
    secrets::get_raw(&format!("jarvis-acc-{account_id}")).filter(|s| !s.trim().is_empty())
}

fn delete_account_secret(account_id: &str) {
    secrets::delete_raw(&format!("jarvis-acc-{account_id}"));
    secrets::delete_raw(&format!("jarvis-acc-refresh-{account_id}"));
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn next_account_id(provider: &str) -> String {
    let seq = ACCOUNT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("acc-{provider}-{}-{seq}", now_secs())
}

fn add_or_activate_account(
    provider: &str,
    label: &str,
    auth_method: &str,
    email: Option<String>,
    token: &str,
    expires_at: Option<u64>,
) -> Result<AiAccount, String> {
    let norm_prov = normalize_provider(provider)?;
    let mut store = load_store();
    for acc in &mut store.accounts {
        if acc.provider == norm_prov {
            acc.active = false;
        }
    }

    let id = next_account_id(&norm_prov);
    set_account_secret(&id, token)?;

    let clean_label = if label.trim().is_empty() {
        let count = store
            .accounts
            .iter()
            .filter(|a| a.provider == norm_prov)
            .count()
            + 1;
        format!("Conta {count}")
    } else {
        label.trim().to_string()
    };

    let account = AiAccount {
        id,
        provider: norm_prov,
        label: clean_label,
        auth_method: auth_method.to_string(),
        email,
        active: true,
        created_at: now_secs(),
        expires_at,
    };
    store.accounts.push(account.clone());
    save_store(&store)?;
    Ok(account)
}

/// Returns the active account's `(token, auth_method)` for a provider,
/// falling back to legacy single-key secrets if no multi-account entry exists.
pub fn get_active_credentials(provider: &str) -> Option<(String, String)> {
    let norm = normalize_provider(provider).ok()?;
    let store = load_store();
    if let Some(acc) = store
        .accounts
        .iter()
        .find(|a| a.provider == norm && a.active)
        .or_else(|| store.accounts.iter().find(|a| a.provider == norm))
    {
        if let Some(tok) = get_account_secret(&acc.id) {
            return Some((tok, acc.auth_method.clone()));
        }
    }

    // Legacy fallback
    match norm.as_str() {
        "groq" => secrets::get("groq-api-key").map(|t| (t, "api_key".into())),
        "gemini" => secrets::get("google-api-key")
            .map(|t| (t, "api_key".into()))
            .or_else(|| secrets::get("gemini-oauth-token").map(|t| (t, "oauth".into()))),
        "openai" => secrets::get("openai-api-key").map(|t| (t, "api_key".into())),
        "claude" => secrets::get("anthropic-api-key")
            .map(|t| (t, "api_key".into()))
            .or_else(|| secrets::get("claude-oauth-token").map(|t| (t, "oauth".into()))),
        _ => None,
    }
}

/// Every saved account of a provider that can be used without a login flow
/// (API keys and tokens), the active one first, with its secret. For failover.
pub fn usable_accounts(provider: &str) -> Vec<(AiAccount, String)> {
    let Ok(norm) = normalize_provider(provider) else { return Vec::new() };
    let mut accs: Vec<AiAccount> = load_store().accounts.into_iter().filter(|a| a.provider == norm && a.auth_method != "oauth").collect();
    accs.sort_by_key(|a| !a.active);
    accs.into_iter().filter_map(|a| get_account_secret(&a.id).map(|t| (a, t))).collect()
}

/// Makes `account_id` the active account of its provider (failover kept it).
pub fn activate(account_id: &str) -> Result<Vec<AiAccount>, String> {
    let mut store = load_store();
    let provider = store.accounts.iter().find(|a| a.id == account_id).map(|a| a.provider.clone()).ok_or("Conta não encontrada.")?;
    for acc in &mut store.accounts {
        if acc.provider == provider {
            acc.active = acc.id == account_id;
        }
    }
    save_store(&store)?;
    Ok(store.accounts)
}

pub fn list_all_accounts() -> Vec<AiAccount> {
    load_store().accounts
}

#[tauri::command]
pub fn jarvis_list_accounts(provider: Option<String>) -> Result<Vec<AiAccount>, String> {
    let accounts = load_store().accounts;
    if let Some(p) = provider.filter(|s| !s.trim().is_empty()) {
        let norm = normalize_provider(&p)?;
        Ok(accounts.into_iter().filter(|a| a.provider == norm).collect())
    } else {
        Ok(accounts)
    }
}

#[tauri::command]
pub fn jarvis_switch_account(app: AppHandle, account_id: String) -> Result<Vec<AiAccount>, String> {
    let mut store = load_store();
    let target_provider = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .map(|a| a.provider.clone())
        .ok_or_else(|| "Conta não encontrada.".to_string())?;

    for acc in &mut store.accounts {
        if acc.provider == target_provider {
            acc.active = acc.id == account_id;
        }
    }
    save_store(&store)?;
    let _ = app.emit("jarvis-accounts-updated", &store.accounts);
    Ok(store.accounts)
}

#[tauri::command]
pub fn jarvis_remove_account(app: AppHandle, account_id: String) -> Result<Vec<AiAccount>, String> {
    let mut store = load_store();
    let idx = store
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| "Conta não encontrada.".to_string())?;
    let removed = store.accounts.remove(idx);
    delete_account_secret(&removed.id);

    if removed.active {
        if let Some(next) = store
            .accounts
            .iter_mut()
            .find(|a| a.provider == removed.provider)
        {
            next.active = true;
        }
    }
    save_store(&store)?;
    let _ = app.emit("jarvis-accounts-updated", &store.accounts);
    Ok(store.accounts)
}

#[tauri::command]
pub fn jarvis_add_account_manual(
    app: AppHandle,
    provider: String,
    label: Option<String>,
    token: String,
    auth_method: Option<String>,
) -> Result<AiAccount, String> {
    let clean_tok = token.trim();
    if clean_tok.is_empty() {
        return Err("Token ou chave não pode ser vazio.".into());
    }
    let method = auth_method.unwrap_or_else(|| "api_key".into());
    let acc = add_or_activate_account(
        &provider,
        label.as_deref().unwrap_or(""),
        &method,
        None,
        clean_tok,
        None,
    )?;
    let _ = app.emit("jarvis-accounts-updated", load_store().accounts);
    Ok(acc)
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[b2 & 0x3f] as char);
        }
    }
    out
}

/// PKCE verifier/challenge (RFC 7636, S256) and an OAuth `state`, all from the
/// OS cryptographic RNG — never from the clock or the process id.
fn generate_pkce() -> Result<(String, String, String), String> {
    let mut verifier_bytes = [0u8; 32];
    let mut state_bytes = [0u8; 16];
    getrandom::getrandom(&mut verifier_bytes)
        .and_then(|_| getrandom::getrandom(&mut state_bytes))
        .map_err(|e| format!("Gerador aleatório do sistema indisponível: {e}"))?;
    let verifier = base64url_encode(&verifier_bytes);

    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = base64url_encode(&h.finalize());

    let state = hex::encode(state_bytes);
    Ok((verifier, challenge, state))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn parse_query_params(raw: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in raw.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(url_decode(k), url_decode(v));
        }
    }
    map
}

struct OAuthConfig {
    auth_url: String,
    token_url: String,
    client_id: String,
    client_secret: Option<String>,
    scope: String,
    extra_auth_params: &'static [(&'static str, &'static str)],
}

pub(crate) fn read_env_or_hkcu(var_name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(var_name) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let script = format!("[Environment]::GetEnvironmentVariable('{var_name}', 'User')");
        if let Ok(out) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .output()
        {
            let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

fn save_refresh_secret(account_id: &str, refresh_token: &str) {
    if refresh_token.trim().is_empty() {
        return;
    }
    let _ = secrets::set_raw(&format!("jarvis-acc-refresh-{account_id}"), refresh_token.trim());
}

fn get_refresh_secret(account_id: &str) -> Option<String> {
    secrets::get_raw(&format!("jarvis-acc-refresh-{account_id}"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Refreshes the active Gemini OAuth access_token using the saved refresh_token.
pub async fn refresh_active_google_token() -> Result<String, String> {
    let mut store = load_store();
    let acc_idx = store
        .accounts
        .iter()
        .position(|a| a.provider == "gemini" && a.active && a.auth_method == "oauth")
        .or_else(|| {
            store
                .accounts
                .iter()
                .position(|a| a.provider == "gemini" && a.auth_method == "oauth")
        })
        .ok_or_else(|| "Nenhuma conta Gemini OAuth encontrada para renovar.".to_string())?;

    let account_id = store.accounts[acc_idx].id.clone();
    let refresh_token = get_refresh_secret(&account_id).ok_or_else(|| {
        "Refresh token ausente. Clique em Entrar no Gemini em Ajustes para reconectar.".to_string()
    })?;

    let cfg = oauth_config_for("gemini").ok_or_else(|| {
        "GOOGLE_OAUTH_CLIENT_ID não configurado para renovar o token.".to_string()
    })?;

    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "refresh_token".to_string()),
        ("client_id", cfg.client_id.clone()),
        ("refresh_token", refresh_token),
    ];
    if let Some(ref sec) = cfg.client_secret {
        form.push(("client_secret", sec.clone()));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let res = client
        .post(&cfg.token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Falha ao renovar token Google: {e}"))?;

    let status = res.status();
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Resposta inválida ao renovar token: {e}"))?;

    if !status.is_success() {
        let msg = body["error_description"]
            .as_str()
            .or_else(|| body["error"].as_str())
            .unwrap_or("Token expirado");
        return Err(format!("Falha ao renovar conta Google ({status}): {msg}"));
    }

    let new_token = body["access_token"]
        .as_str()
        .ok_or_else(|| "access_token ausente na renovação".to_string())?
        .to_string();
    let new_expires = body["expires_in"].as_u64().map(|sec| now_secs() + sec);

    set_account_secret(&account_id, &new_token)?;
    store.accounts[acc_idx].expires_at = new_expires;
    let _ = save_store(&store);

    Ok(new_token)
}

/// The user's Google OAuth client (the one Gemini's login uses), for Google
/// Calendar: (client id, client secret if any).
pub(crate) fn google_client() -> Option<(String, Option<String>)> {
    oauth_config_for("gemini").map(|c| (c.client_id, c.client_secret))
}

fn oauth_config_for(provider: &str) -> Option<OAuthConfig> {
    match provider {
        "gemini" => {
            let cid = secrets::get("gemini-oauth-client-id")
                .or_else(|| read_env_or_hkcu("GOOGLE_OAUTH_CLIENT_ID"))
                .or_else(|| read_env_or_hkcu("JARVIS_GOOGLE_CLIENT_ID"))?;
            let csec = secrets::get("gemini-oauth-client-secret")
                .or_else(|| read_env_or_hkcu("GOOGLE_OAUTH_CLIENT_SECRET"))
                .or_else(|| read_env_or_hkcu("JARVIS_GOOGLE_CLIENT_SECRET"));
            Some(OAuthConfig {
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                client_id: cid,
                client_secret: csec,
                scope: "https://www.googleapis.com/auth/generative-language.retriever openid email".into(),
                extra_auth_params: &[("access_type", "offline"), ("prompt", "consent")],
            })
        }
        "openai" => {
            let cid = secrets::get("openai-oauth-client-id")
                .or_else(|| read_env_or_hkcu("OPENAI_OAUTH_CLIENT_ID"))
                .or_else(|| read_env_or_hkcu("JARVIS_OPENAI_CLIENT_ID"))?;
            Some(OAuthConfig {
                auth_url: "https://auth.openai.com/oauth/authorize".into(),
                token_url: "https://auth.openai.com/oauth/token".into(),
                client_id: cid,
                client_secret: None,
                scope: "openid profile email offline_access".into(),
                extra_auth_params: &[],
            })
        }
        "claude" => {
            let cid = secrets::get("claude-oauth-client-id")
                .or_else(|| read_env_or_hkcu("CLAUDE_OAUTH_CLIENT_ID"))
                .or_else(|| read_env_or_hkcu("JARVIS_CLAUDE_CLIENT_ID"))?;
            Some(OAuthConfig {
                auth_url: "https://claude.ai/oauth/authorize".into(),
                token_url: "https://console.anthropic.com/v1/oauth/token".into(),
                client_id: cid,
                client_secret: None,
                scope: "user:inference user:profile".into(),
                extra_auth_params: &[],
            })
        }
        _ => None,
    }
}

fn provider_portal_info(provider: &str) -> (&'static str, &'static str, &'static str) {
    match provider {
        "claude" => (
            "Claude (Anthropic)",
            "https://console.anthropic.com/settings/keys",
            "sk-ant-... ou token OAuth",
        ),
        "gemini" => (
            "Google Gemini",
            "https://aistudio.google.com/apikey",
            "AIza... ou token OAuth Google",
        ),
        "openai" => (
            "OpenAI / ChatGPT",
            "https://platform.openai.com/api-keys",
            "sk-... ou token de sessão",
        ),
        "groq" => (
            "Groq",
            "https://console.groq.com/keys",
            "gsk_...",
        ),
        _ => ("IA", "https://google.com", "Token"),
    }
}

fn open_browser_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| format!("Falha ao abrir navegador: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Falha ao abrir navegador: {e}"))?;
        Ok(())
    }
}

fn render_login_html(provider: &str, default_label: &str) -> String {
    let (title, console_url, placeholder) = provider_portal_info(provider);
    let safe_label = default_label.replace('"', "&quot;").replace('<', "&lt;");
    format!(
        r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Conectar {title} — Jarvis</title>
<style>
  body {{ margin:0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background:#0b0c10; color:#e6e8ee; display:flex; align-items:center; justify-content:center; min-height:100vh; }}
  .card {{ width:100%; max-width:440px; background:#14161d; border:1px solid #262936; border-radius:14px; padding:28px; box-shadow:0 20px 60px rgba(0,0,0,.45); }}
  h1 {{ font-size:19px; margin:0 0 6px; font-weight:600; }}
  p {{ font-size:13px; color:#9aa1b2; margin:0 0 18px; line-height:1.5; }}
  label {{ display:block; font-size:12px; color:#b8bfd0; margin:12px 0 6px; }}
  input {{ width:100%; box-sizing:border-box; padding:10px 12px; border-radius:8px; border:1px solid #2d3242; background:#0d0f14; color:#f4f6fb; font-size:13px; outline:none; }}
  input:focus {{ border-color:#5b8cff; }}
  .btn {{ display:inline-flex; align-items:center; justify-content:center; width:100%; padding:11px 14px; border-radius:9px; border:none; font-size:13px; font-weight:600; cursor:pointer; text-decoration:none; box-sizing:border-box; margin-top:12px; }}
  .btn-primary {{ background:#3b82f6; color:#fff; }}
  .btn-primary:hover {{ background:#2563eb; }}
  .btn-ghost {{ background:#1d202b; color:#d4d8e3; border:1px solid #2d3242; margin-top:0; margin-bottom:14px; }}
  .btn-ghost:hover {{ background:#252938; }}
</style>
</head>
<body>
  <div class="card">
    <h1>Conectar {title}</h1>
    <p>Autentique sua conta para adicionar ao Jarvis. Você pode conectar múltiplas contas e alternar entre elas nos Ajustes.</p>
    <a class="btn btn-ghost" href="{console_url}" target="_blank" rel="noopener">Abrir portal {title} no navegador ↗</a>
    <form method="POST" action="/submit">
      <label>Nome da conta (ex: Pessoal, Trabalho)</label>
      <input name="label" value="{safe_label}" placeholder="Conta principal" />
      <label>Token de sessão ou Chave API</label>
      <input name="token" type="password" required autofocus placeholder="{placeholder}" />
      <button class="btn btn-primary" type="submit">Conectar ao Jarvis</button>
    </form>
  </div>
</body>
</html>"#
    )
}

fn render_result_html(title: &str, message: &str, ok: bool) -> String {
    let accent = if ok { "#22c55e" } else { "#ef4444" };
    format!(
        r#"<!DOCTYPE html>
<html lang="pt-BR">
<head><meta charset="utf-8" /><title>{title} — Jarvis</title>
<style>
  body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:#0b0c10; color:#e6e8ee; display:flex; align-items:center; justify-content:center; min-height:100vh; }}
  .card {{ max-width:400px; background:#14161d; border:1px solid #262936; border-radius:14px; padding:28px; text-align:center; }}
  .dot {{ width:12px; height:12px; border-radius:50%; background:{accent}; margin:0 auto 14px; }}
  h1 {{ font-size:18px; margin:0 0 8px; }}
  p {{ font-size:13px; color:#9aa1b2; margin:0; line-height:1.5; }}
</style></head>
<body><div class="card"><div class="dot"></div><h1>{title}</h1><p>{message}</p></div></body></html>"#
    )
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    html: &str,
) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Starts a local callback server on `127.0.0.1:<port>`, opens the browser
/// (using OAuth PKCE when configured, or the Jarvis local connect portal),
/// waits for authentication, saves the new account, and returns it.
#[tauri::command]
pub async fn jarvis_login_account(
    app: AppHandle,
    provider: String,
    label: Option<String>,
) -> Result<AiAccount, String> {
    let norm_prov = normalize_provider(&provider)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Falha ao iniciar servidor de login local: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Porta local indisponível: {e}"))?
        .port();

    let (verifier, challenge, state) = generate_pkce()?;
    let oauth_cfg = oauth_config_for(&norm_prov);
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let launch_url = if let Some(ref cfg) = oauth_cfg {
        let mut url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            cfg.auth_url,
            url_encode(&cfg.client_id),
            url_encode(&redirect_uri),
            url_encode(&cfg.scope),
            url_encode(&state),
            url_encode(&challenge),
        );
        for (k, v) in cfg.extra_auth_params {
            url.push_str(&format!("&{}={}", url_encode(k), url_encode(v)));
        }
        url
    } else {
        format!("http://127.0.0.1:{port}/login")
    };

    open_browser_url(&launch_url)?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let default_label = label.unwrap_or_default();

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("Tempo limite de login expirado (3 min). Tente novamente.".into());
        }

        let accept_res = tokio::time::timeout(remaining, listener.accept()).await;
        let (mut stream, _) = match accept_res {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(format!("Erro no callback local: {e}")),
            Err(_) => return Err("Tempo limite de login expirado (3 min).".into()),
        };

        let mut buf = vec![0u8; 16384];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            continue;
        }
        let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
        let first_line = req_str.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path_and_query = parts.next().unwrap_or("/");
        let (path, query) = path_and_query.split_once('?').unwrap_or((path_and_query, ""));

        if method == "GET" && path == "/login" {
            let html = render_login_html(&norm_prov, &default_label);
            write_http_response(&mut stream, "200 OK", &html).await;
            continue;
        }

        if method == "POST" && path == "/submit" {
            let body = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
            let params = parse_query_params(body);
            let token = params.get("token").map(String::as_str).unwrap_or("").trim();
            let chosen_label = params
                .get("label")
                .map(String::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(default_label.as_str());

            if token.is_empty() {
                let html = render_result_html("Erro", "Token vazio. Volte e tente novamente.", false);
                write_http_response(&mut stream, "400 Bad Request", &html).await;
                continue;
            }

            let auth_method = if token.starts_with("eyJ") {
                "oauth"
            } else {
                "api_key"
            };
            let acc = add_or_activate_account(
                &norm_prov,
                chosen_label,
                auth_method,
                None,
                token,
                None,
            )?;
            let _ = app.emit("jarvis-accounts-updated", load_store().accounts);
            let html = render_result_html(
                "Conta conectada!",
                "Autenticação concluída com sucesso. Você já pode fechar esta aba e voltar ao Jarvis.",
                true,
            );
            write_http_response(&mut stream, "200 OK", &html).await;
            return Ok(acc);
        }

        if method == "GET" && path == "/callback" {
            let params = parse_query_params(query);
            if let Some(err) = params.get("error") {
                let html = render_result_html("Login cancelado", err, false);
                write_http_response(&mut stream, "400 Bad Request", &html).await;
                return Err(format!("OAuth recusado: {err}"));
            }
            if params.get("state").map(String::as_str) != Some(state.as_str()) {
                let html = render_result_html("Erro de segurança", "State OAuth inválido.", false);
                write_http_response(&mut stream, "400 Bad Request", &html).await;
                return Err("State OAuth inválido.".into());
            }
            let Some(code) = params.get("code") else {
                let html = render_result_html("Erro", "Código OAuth ausente.", false);
                write_http_response(&mut stream, "400 Bad Request", &html).await;
                continue;
            };
            let Some(ref cfg) = oauth_cfg else {
                return Err("Configuração OAuth ausente.".into());
            };

            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())?;
            let mut form_params: Vec<(&str, &str)> = vec![
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("client_id", cfg.client_id.as_str()),
                ("code_verifier", verifier.as_str()),
            ];
            if let Some(ref csec) = cfg.client_secret {
                form_params.push(("client_secret", csec.as_str()));
            }
            let res = client
                .post(&cfg.token_url)
                .form(&form_params)
                .send()
                .await
                .map_err(|e| format!("Falha ao trocar token OAuth: {e}"))?;

            let status = res.status();
            let body: serde_json::Value = res
                .json()
                .await
                .map_err(|e| format!("Resposta OAuth inválida: {e}"))?;
            if !status.is_success() {
                let msg = body["error_description"]
                    .as_str()
                    .or_else(|| body["error"].as_str())
                    .unwrap_or("Erro no servidor OAuth");
                let html = render_result_html("Erro OAuth", msg, false);
                write_http_response(&mut stream, "400 Bad Request", &html).await;
                return Err(format!("OAuth ({status}): {msg}"));
            }

            let access_token = body["access_token"]
                .as_str()
                .ok_or_else(|| "access_token ausente na resposta OAuth".to_string())?;
            let refresh_token = body["refresh_token"].as_str().unwrap_or("");
            let expires_in = body["expires_in"].as_u64();
            let expires_at = expires_in.map(|sec| now_secs() + sec);

            let mut user_email: Option<String> = None;
            if norm_prov == "gemini" {
                if let Ok(info_res) = client
                    .get("https://www.googleapis.com/oauth2/v2/userinfo")
                    .bearer_auth(access_token)
                    .send()
                    .await
                {
                    if let Ok(info) = info_res.json::<serde_json::Value>().await {
                        user_email = info["email"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string);
                    }
                }
            }

            let chosen_label = if default_label.trim().is_empty() {
                user_email.clone().unwrap_or_default()
            } else {
                default_label.clone()
            };

            let acc = add_or_activate_account(
                &norm_prov,
                &chosen_label,
                "oauth",
                user_email,
                access_token,
                expires_at,
            )?;
            if !refresh_token.is_empty() {
                save_refresh_secret(&acc.id, refresh_token);
            }
            let _ = app.emit("jarvis-accounts-updated", load_store().accounts);
            let html = render_result_html(
                "Conta conectada!",
                "Autenticação concluída com sucesso. Você já pode fechar esta aba e voltar ao Jarvis.",
                true,
            );
            write_http_response(&mut stream, "200 OK", &html).await;
            return Ok(acc);
        }

        write_http_response(&mut stream, "404 Not Found", "Not Found").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_values_are_random_and_well_formed() {
        let (v1, c1, s1) = generate_pkce().unwrap();
        let (v2, c2, s2) = generate_pkce().unwrap();
        assert_ne!(v1, v2);
        assert_ne!(s1, s2);
        assert_ne!(c1, c2);
        // RFC 7636: 43-128 chars from the unreserved set.
        assert!((43..=128).contains(&v1.len()));
        assert!(v1.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)));
        let mut h = Sha256::new();
        h.update(v1.as_bytes());
        assert_eq!(c1, base64url_encode(&h.finalize()));
        assert_eq!(s1.len(), 32);
    }
}
