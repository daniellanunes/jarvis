//! Google Calendar for Jarvis: login with the user's own Google OAuth client
//! (the one the Gemini login uses, a "Desktop app" client, loopback redirect),
//! list events across the calendars shown in Google Calendar, create, move,
//! rename and delete events. Times the brain and the UI send are local wall
//! clock ("2026-11-20T15:30"); the local UTC offset is attached here.
//!
//! The refresh token lives in the OS keychain (`google-calendar-refresh-token`).
//! While the user's Google Cloud app is in "Testing", Google expires it after
//! 7 days and the panel asks to connect again.

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::secrets;

const API: &str = "https://www.googleapis.com/calendar/v3";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const REFRESH_KEY: &str = "google-calendar-refresh-token";
const EMAIL_KEY: &str = "google-calendar-email";
/// Gmail is read and drafted only when the user asks (gmail.rs); compose
/// also covers sending a draft, which always waits for Permitir.
const SCOPES: &str = "openid email https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose";

struct Token {
    access: String,
    expires: Instant,
}

static TOKEN: LazyLock<Mutex<Option<Token>>> = LazyLock::new(|| Mutex::new(None));

pub(crate) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

pub fn connected() -> bool {
    secrets::present(REFRESH_KEY)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    /// A Google OAuth client id is available (settings, keychain or env).
    pub configured: bool,
    pub connected: bool,
    pub email: String,
    /// Google Cloud project number of the OAuth client in use (the client
    /// id's prefix), so the user can open exactly that project's settings.
    pub project: String,
}

/// "206533512741-abc.apps.googleusercontent.com" → "206533512741".
fn project_number(client_id: &str) -> String {
    client_id
        .split('-')
        .next()
        .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("")
        .to_string()
}

#[tauri::command]
pub fn calendar_status() -> CalendarStatus {
    let client = crate::jarvis_accounts::google_client();
    CalendarStatus {
        configured: client.is_some(),
        connected: connected(),
        email: secrets::get(EMAIL_KEY).unwrap_or_default(),
        project: client.map(|(id, _)| project_number(&id)).unwrap_or_default(),
    }
}

// ── Login ─────────────────────────────────────────────────────────────────────

pub(crate) fn b64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if c.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if c.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

fn query_param(path: &str, key: &str) -> Option<String> {
    let url = reqwest::Url::parse(&format!("http://x{path}")).ok()?;
    url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.to_string())
}

fn page(title: &str, msg: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
<body style=\"font:15px system-ui;background:#141416;color:#ececee;display:grid;place-items:center;height:100vh;margin:0\">\
<div style=\"max-width:440px;text-align:center\"><h2 style=\"font-weight:600\">{title}</h2><p style=\"color:#a8a8b0\">{msg}</p></div>"
    )
}

async fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    let res = client()?
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Google: falha de conexão ({e})"))?;
    let status = res.status();
    let body: Value = res.json().await.map_err(|e| format!("Google: resposta inválida ({e})"))?;
    if !status.is_success() {
        let err = body["error"].as_str().unwrap_or("");
        if err == "invalid_grant" {
            // Revoked, or expired (7 days while the Cloud app is in Testing).
            let _ = secrets::clear(REFRESH_KEY);
            return Err("A conexão com o Google Agenda expirou. Conecte de novo na aba Agenda.".into());
        }
        let msg = body["error_description"].as_str().unwrap_or(err);
        return Err(format!("Google ({status}): {msg}"));
    }
    Ok(body)
}

fn keep_access(body: &Value) -> Result<String, String> {
    let access = body["access_token"].as_str().ok_or("Google não devolveu access_token")?.to_string();
    let secs = body["expires_in"].as_u64().unwrap_or(3600);
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Token {
        access: access.clone(),
        expires: Instant::now() + Duration::from_secs(secs.saturating_sub(60)),
    });
    Ok(access)
}

/// Opens Google's consent page and waits (up to 3 min) for the loopback redirect.
#[tauri::command]
pub async fn calendar_login() -> Result<String, String> {
    let (cid, secret) = crate::jarvis_accounts::google_client().ok_or(
        "Nenhum cliente OAuth do Google configurado (GOOGLE_OAUTH_CLIENT_ID). Veja os passos na aba Agenda.",
    )?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    crate::log::line(format!("calendar login: Google Cloud project {}", project_number(&cid)));

    let mut verifier = [0u8; 48];
    let mut state = [0u8; 16];
    getrandom::getrandom(&mut verifier)
        .and_then(|_| getrandom::getrandom(&mut state))
        .map_err(|e| format!("RNG indisponível: {e}"))?;
    let verifier = b64url(&verifier);
    let state = hex::encode(state);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));

    let mut url = reqwest::Url::parse(AUTH_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &cid)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    crate::jarvis_pc::jarvis_open_target(url.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (mut stream, _) = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| "Tempo esgotado esperando o login do Google.".to_string())?
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        if !path.contains("state=") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let reply = |ok: bool, msg: &str| {
            let html = page(if ok { "Google Agenda conectado" } else { "Login não concluído" }, msg);
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            )
        };
        if query_param(&path, "state").as_deref() != Some(state.as_str()) {
            let _ = stream.write_all(reply(false, "State inválido. Tente de novo.").as_bytes()).await;
            return Err("Login do Google recusado (state inválido).".into());
        }
        if let Some(err) = query_param(&path, "error") {
            let hint = if err == "access_denied" {
                "Acesso negado. Se o Google disse que o app não foi verificado, adicione seu e-mail em Usuários de teste no projeto do Google Cloud indicado na aba Agenda."
            } else {
                err.as_str()
            };
            let _ = stream.write_all(reply(false, hint).as_bytes()).await;
            return Err(format!("Login do Google cancelado: {hint}"));
        }
        let code = query_param(&path, "code").ok_or("Google não devolveu o código.")?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("client_id", cid.as_str()),
            ("code_verifier", verifier.as_str()),
        ];
        if let Some(s) = secret.as_deref() {
            form.push(("client_secret", s));
        }
        let result = async {
            let body = token_request(&form).await?;
            let refresh = body["refresh_token"]
                .as_str()
                .ok_or("O Google não devolveu um refresh token. Remova o acesso do Jarvis em myaccount.google.com/permissions e conecte de novo.")?;
            secrets::set(REFRESH_KEY, refresh)?;
            let access = keep_access(&body)?;
            if let Ok(r) = client()?.get(USERINFO_URL).bearer_auth(&access).send().await {
                if let Ok(info) = r.json::<Value>().await {
                    if let Some(email) = info["email"].as_str() {
                        let _ = secrets::set(EMAIL_KEY, email);
                    }
                }
            }
            Ok::<(), String>(())
        }
        .await;
        return match result {
            Ok(()) => {
                let _ = stream.write_all(reply(true, "Pode fechar esta aba e voltar para o Jarvis.").as_bytes()).await;
                Ok("Google Agenda conectado.".into())
            }
            Err(e) => {
                let _ = stream.write_all(reply(false, &e).as_bytes()).await;
                Err(e)
            }
        };
    }
}

#[tauri::command]
pub fn calendar_logout() -> Result<(), String> {
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let _ = secrets::clear(EMAIL_KEY);
    secrets::clear(REFRESH_KEY)
}

pub(crate) async fn access_token(force: bool) -> Result<String, String> {
    if !force {
        if let Some(t) = TOKEN.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if Instant::now() < t.expires {
                return Ok(t.access.clone());
            }
        }
    }
    let refresh = secrets::get(REFRESH_KEY).ok_or("Google Agenda não conectado. Conecte na aba Agenda.")?;
    let (cid, secret) = crate::jarvis_accounts::google_client().ok_or("Cliente OAuth do Google não configurado.")?;
    let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", cid.as_str())];
    if let Some(s) = secret.as_deref() {
        form.push(("client_secret", s));
    }
    let body = token_request(&form).await?;
    keep_access(&body)
}

async fn call(method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Option<Value>, String> {
    let mut refreshed = false;
    loop {
        let token = access_token(refreshed).await?;
        let mut req = client()?.request(method.clone(), format!("{API}{path}")).bearer_auth(&token);
        if let Some(b) = &body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|e| format!("Google Agenda: falha de conexão ({e})"))?;
        let status = res.status().as_u16();
        if status == 401 && !refreshed {
            refreshed = true;
            continue;
        }
        let text = res.text().await.unwrap_or_default();
        if (200..300).contains(&status) {
            return Ok(if text.trim().is_empty() { None } else { serde_json::from_str(&text).ok() });
        }
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let msg = v["error"]["message"].as_str().unwrap_or(text.as_str());
        if status == 403 && (msg.contains("has not been used") || msg.contains("is disabled")) {
            return Err("A Google Calendar API não está ativada no seu projeto do Google Cloud. Ative em console.cloud.google.com/apis/library/calendar-json.googleapis.com e tente de novo.".into());
        }
        return Err(format!("Google Agenda ({status}): {msg}"));
    }
}

fn enc(s: &str) -> String {
    let mut u = reqwest::Url::parse("http://x/").unwrap();
    u.path_segments_mut().unwrap().push(s);
    u.path().trim_start_matches('/').to_string()
}

// ── Local time ────────────────────────────────────────────────────────────────

const WEEKDAYS: [&str; 7] = ["segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado", "domingo"];

/// "quinta-feira, 20/11/2026 14:03 (UTC-03:00)" — for the brain's prompt.
pub fn now_context() -> String {
    let now = Local::now();
    format!(
        "{}, {:02}/{:02}/{} {:02}:{:02} (UTC{})",
        WEEKDAYS[now.weekday().num_days_from_monday() as usize],
        now.day(),
        now.month(),
        now.year(),
        now.hour(),
        now.minute(),
        now.format("%:z")
    )
}

/// Local wall-clock "YYYY-MM-DDTHH:MM[:SS]" → RFC 3339 with the local offset.
pub fn local_to_rfc3339(s: &str) -> Result<String, String> {
    let s = s.trim().replace(' ', "T");
    let naive = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M"))
        .map_err(|_| format!("Data/hora inválida: {s} (use AAAA-MM-DDTHH:MM)"))?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.to_rfc3339())
        .ok_or_else(|| format!("Horário inexistente no fuso local: {s}"))
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| format!("Data inválida: {s} (use AAAA-MM-DD)"))
}

fn day_start(d: NaiveDate) -> String {
    local_to_rfc3339(&format!("{}T00:00", d.format("%Y-%m-%d"))).unwrap_or_default()
}

// ── Events ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    /// "calendarId|eventId": everything needed to change or delete it.
    pub key: String,
    pub title: String,
    /// RFC 3339 for timed events, "YYYY-MM-DD" for all-day ones.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub calendar: String,
    pub color: String,
    pub link: String,
    pub meet: String,
    /// Popup reminders, in minutes before the start (the calendar's defaults
    /// when the event uses them). The Jarvis alerts at these times too.
    #[serde(default)]
    pub reminders: Vec<i64>,
}

impl CalEvent {
    /// End in local wall-clock time (all-day events end at the next midnight).
    pub fn end_local(&self) -> NaiveDateTime {
        if self.all_day {
            return NaiveDate::parse_from_str(&self.end, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .unwrap_or_else(|| Local::now().naive_local());
        }
        chrono::DateTime::parse_from_rfc3339(&self.end).map(|d| d.with_timezone(&Local).naive_local()).unwrap_or_else(|_| Local::now().naive_local())
    }

    /// "14:30" for timed events, "" for all-day ones.
    pub fn start_hm(&self) -> String {
        if self.all_day {
            return String::new();
        }
        chrono::DateTime::parse_from_rfc3339(&self.start).map(|d| d.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
struct CalendarInfo {
    id: String,
    name: String,
    color: String,
    writable: bool,
    primary: bool,
    /// The calendar's default popup reminders (minutes before).
    default_reminders: Vec<i64>,
}

/// Popup minutes from a `reminders` object ({useDefault, overrides}).
pub fn popup_minutes(reminders: &Value, defaults: &[i64]) -> Vec<i64> {
    if reminders["useDefault"].as_bool().unwrap_or(true) {
        return defaults.to_vec();
    }
    reminders["overrides"]
        .as_array()
        .map(|a| a.iter().filter(|r| r["method"] == "popup").filter_map(|r| r["minutes"].as_i64()).collect())
        .unwrap_or_default()
}

async fn calendars() -> Result<Vec<CalendarInfo>, String> {
    let v = call(reqwest::Method::GET, "/users/me/calendarList?minAccessRole=reader", None)
        .await?
        .unwrap_or(Value::Null);
    Ok(v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                // Only the calendars shown (ticked) in Google Calendar.
                .filter(|c| c["selected"].as_bool().unwrap_or(false) || c["primary"].as_bool().unwrap_or(false))
                .map(|c| CalendarInfo {
                    id: c["id"].as_str().unwrap_or("").into(),
                    name: c["summaryOverride"].as_str().or(c["summary"].as_str()).unwrap_or("").into(),
                    color: c["backgroundColor"].as_str().unwrap_or("#8e8e96").into(),
                    writable: matches!(c["accessRole"].as_str(), Some("owner" | "writer")),
                    primary: c["primary"].as_bool().unwrap_or(false),
                    default_reminders: c["defaultReminders"]
                        .as_array()
                        .map(|a| a.iter().filter(|r| r["method"] == "popup").filter_map(|r| r["minutes"].as_i64()).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default())
}

fn event_of(cal: &CalendarInfo, e: &Value) -> Option<CalEvent> {
    if e["status"] == "cancelled" {
        return None;
    }
    let all_day = e["start"]["date"].is_string();
    let pick = |k: &str| e[k]["dateTime"].as_str().or(e[k]["date"].as_str()).unwrap_or("").to_string();
    Some(CalEvent {
        key: format!("{}|{}", cal.id, e["id"].as_str()?),
        title: e["summary"].as_str().unwrap_or("(sem título)").into(),
        start: pick("start"),
        end: pick("end"),
        all_day,
        location: e["location"].as_str().unwrap_or("").into(),
        description: e["description"].as_str().unwrap_or("").chars().take(400).collect(),
        calendar: cal.name.clone(),
        color: cal.color.clone(),
        link: e["htmlLink"].as_str().unwrap_or("").into(),
        meet: e["hangoutLink"].as_str().unwrap_or("").into(),
        reminders: popup_minutes(&e["reminders"], &cal.default_reminders),
    })
}

/// Events from `from` (local date) for `days` days, across the shown calendars, by start.
pub async fn events(from: NaiveDate, days: u32) -> Result<Vec<CalEvent>, String> {
    let time_min = day_start(from);
    // Up to a bit over a year, for the Year view.
    let time_max = day_start(from + ChronoDuration::days(days.clamp(1, 400) as i64));
    let cals = calendars().await?;
    let mut out = Vec::new();
    for cal in &cals {
        let mut page_token: Option<String> = None;
        // Follow Google's pages (2500 events each), capped so a runaway
        // calendar cannot stall the panel.
        for _ in 0..8 {
            let mut q = reqwest::Url::parse("http://x/").unwrap();
            q.query_pairs_mut()
                .append_pair("timeMin", &time_min)
                .append_pair("timeMax", &time_max)
                .append_pair("singleEvents", "true")
                .append_pair("orderBy", "startTime")
                .append_pair("maxResults", "2500");
            if let Some(t) = &page_token {
                q.query_pairs_mut().append_pair("pageToken", t);
            }
            let path = format!("/calendars/{}/events?{}", enc(&cal.id), q.query().unwrap_or(""));
            match call(reqwest::Method::GET, &path, None).await {
                Ok(Some(v)) => {
                    if let Some(items) = v["items"].as_array() {
                        out.extend(items.iter().filter_map(|e| event_of(cal, e)));
                    }
                    page_token = v["nextPageToken"].as_str().map(str::to_string);
                }
                Ok(None) => page_token = None,
                // One unreadable shared calendar must not hide the others.
                Err(e) if !cal.primary => {
                    crate::log::line(format!("calendar {}: {e}", cal.name));
                    page_token = None;
                }
                Err(e) => return Err(e),
            }
            if page_token.is_none() {
                break;
            }
        }
    }
    out.sort_by(|a, b| sort_key(&a.start).cmp(&sort_key(&b.start)));
    Ok(out)
}

/// All-day events ("2026-11-20") sort before timed ones that day.
fn sort_key(start: &str) -> String {
    if start.len() == 10 {
        format!("{start}T00:00:00-99")
    } else {
        chrono::DateTime::parse_from_rfc3339(start)
            .map(|d| d.with_timezone(&Local).format("%Y-%m-%dT%H:%M:%S").to_string())
            .unwrap_or_else(|_| start.to_string())
    }
}

#[tauri::command]
pub async fn calendar_events(from: String, days: u32) -> Result<Vec<CalEvent>, String> {
    events(parse_date(&from)?, days).await
}

fn split_key(key: &str) -> Result<(String, String), String> {
    key.split_once('|')
        .map(|(c, e)| (c.to_string(), e.to_string()))
        .ok_or_else(|| format!("Evento inválido: {key}"))
}

/// The event body for create/update. `start`/`end` are local wall clock;
/// all-day events take dates and Google's end date is exclusive.
fn when(start: &str, end: Option<&str>, minutes: Option<i64>, all_day: bool) -> Result<(Value, Value), String> {
    if all_day {
        let d = parse_date(&start[..start.len().min(10)])?;
        let last = match end {
            Some(e) if !e.is_empty() => parse_date(&e[..e.len().min(10)])?,
            _ => d,
        };
        let excl = last.max(d) + ChronoDuration::days(1);
        return Ok((json!({ "date": d.format("%Y-%m-%d").to_string() }), json!({ "date": excl.format("%Y-%m-%d").to_string() })));
    }
    let s = local_to_rfc3339(start)?;
    let e = match end.filter(|e| !e.trim().is_empty()) {
        Some(e) => local_to_rfc3339(e)?,
        None => {
            let st = chrono::DateTime::parse_from_rfc3339(&s).map_err(|e| e.to_string())?;
            (st + ChronoDuration::minutes(minutes.unwrap_or(60).max(5))).to_rfc3339()
        }
    };
    let (a, b) = (chrono::DateTime::parse_from_rfc3339(&s), chrono::DateTime::parse_from_rfc3339(&e));
    if let (Ok(a), Ok(b)) = (a, b) {
        if b <= a {
            return Err("O fim do evento precisa ser depois do início.".into());
        }
    }
    Ok((json!({ "dateTime": s }), json!({ "dateTime": e })))
}

pub struct NewEvent<'a> {
    pub title: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    pub minutes: Option<i64>,
    pub all_day: bool,
    pub location: Option<&'a str>,
    pub description: Option<&'a str>,
    /// Notify this many minutes before (Google popup + the Jarvis's alert).
    pub remind: Option<i64>,
}

/// Google's reminder override for "notify N minutes before".
fn reminder_body(minutes: i64) -> Value {
    json!({ "useDefault": false, "overrides": [{ "method": "popup", "minutes": minutes.clamp(0, 40320) }] })
}

pub async fn create(ev: NewEvent<'_>) -> Result<CalEvent, String> {
    if ev.title.trim().is_empty() {
        return Err("O evento precisa de um título.".into());
    }
    let (start, end) = when(ev.start, ev.end, ev.minutes, ev.all_day)?;
    let mut body = json!({ "summary": ev.title.trim(), "start": start, "end": end });
    if let Some(l) = ev.location.filter(|l| !l.trim().is_empty()) {
        body["location"] = json!(l.trim());
    }
    if let Some(d) = ev.description.filter(|d| !d.trim().is_empty()) {
        body["description"] = json!(d.trim());
    }
    if let Some(m) = ev.remind {
        body["reminders"] = reminder_body(m);
    }
    let cals = calendars().await?;
    let cal = cals
        .iter()
        .find(|c| c.primary)
        .or_else(|| cals.iter().find(|c| c.writable))
        .cloned()
        .unwrap_or(CalendarInfo { id: "primary".into(), name: "Agenda".into(), color: "#8e8e96".into(), writable: true, primary: true, default_reminders: Vec::new() });
    let v = call(reqwest::Method::POST, &format!("/calendars/{}/events", enc(&cal.id)), Some(body))
        .await?
        .ok_or("O Google não devolveu o evento criado.")?;
    event_of(&cal, &v).ok_or_else(|| "Evento criado, mas a resposta veio incompleta.".into())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn calendar_create(
    title: String,
    start: String,
    end: Option<String>,
    all_day: Option<bool>,
    location: Option<String>,
    description: Option<String>,
    remind: Option<i64>,
) -> Result<CalEvent, String> {
    create(NewEvent {
        title: &title,
        start: &start,
        end: end.as_deref(),
        minutes: None,
        all_day: all_day.unwrap_or(false),
        location: location.as_deref(),
        description: description.as_deref(),
        remind,
    })
    .await
}

/// Renames and/or moves an event. A new start without an end keeps the duration.
pub async fn update(key: &str, title: Option<&str>, start: Option<&str>, end: Option<&str>, remind: Option<i64>) -> Result<String, String> {
    let (cal, id) = split_key(key)?;
    let path = format!("/calendars/{}/events/{}", enc(&cal), enc(&id));
    let mut patch = json!({});
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        patch["summary"] = json!(t.trim());
    }
    if let Some(s) = start.filter(|s| !s.trim().is_empty()) {
        let current = call(reqwest::Method::GET, &path, None).await?.unwrap_or(Value::Null);
        let all_day = current["start"]["date"].is_string();
        let minutes = if all_day {
            None
        } else {
            let a = chrono::DateTime::parse_from_rfc3339(current["start"]["dateTime"].as_str().unwrap_or(""));
            let b = chrono::DateTime::parse_from_rfc3339(current["end"]["dateTime"].as_str().unwrap_or(""));
            match (a, b) {
                (Ok(a), Ok(b)) => Some((b - a).num_minutes()),
                _ => None,
            }
        };
        let (st, en) = when(s, end, minutes, all_day)?;
        patch["start"] = st;
        patch["end"] = en;
    }
    if let Some(m) = remind {
        patch["reminders"] = reminder_body(m);
    }
    if patch.as_object().map_or(true, |o| o.is_empty()) {
        return Err("Nada para alterar.".into());
    }
    let v = call(reqwest::Method::PATCH, &path, Some(patch)).await?.unwrap_or(Value::Null);
    Ok(format!("Evento atualizado: {}", v["summary"].as_str().unwrap_or("")))
}

#[tauri::command]
pub async fn calendar_update(key: String, title: Option<String>, start: Option<String>, end: Option<String>) -> Result<String, String> {
    update(&key, title.as_deref(), start.as_deref(), end.as_deref(), None).await
}

pub async fn delete(key: &str) -> Result<(), String> {
    let (cal, id) = split_key(key)?;
    call(reqwest::Method::DELETE, &format!("/calendars/{}/events/{}", enc(&cal), enc(&id)), None).await?;
    Ok(())
}

#[tauri::command]
pub async fn calendar_delete(key: String) -> Result<(), String> {
    delete(&key).await
}

/// A readable agenda for the brain and spoken replies:
/// "quinta 20/11\n- 15:30–16:30 Reunião (Trabalho) [id: …]".
pub fn describe(events: &[CalEvent], with_ids: bool) -> String {
    let mut out = String::new();
    let mut last_day = String::new();
    for e in events {
        let (day, span) = if e.all_day {
            (e.start.clone(), "dia todo".to_string())
        } else {
            let s = chrono::DateTime::parse_from_rfc3339(&e.start).map(|d| d.with_timezone(&Local));
            let en = chrono::DateTime::parse_from_rfc3339(&e.end).map(|d| d.with_timezone(&Local));
            match (s, en) {
                (Ok(s), Ok(en)) => (s.format("%Y-%m-%d").to_string(), format!("{}–{}", s.format("%H:%M"), en.format("%H:%M"))),
                _ => (e.start.chars().take(10).collect(), e.start.clone()),
            }
        };
        if day != last_day {
            if let Ok(d) = parse_date(&day) {
                out.push_str(&format!(
                    "{}{} {:02}/{:02}\n",
                    if out.is_empty() { "" } else { "\n" },
                    WEEKDAYS[d.weekday().num_days_from_monday() as usize],
                    d.day(),
                    d.month()
                ));
            }
            last_day = day;
        }
        out.push_str(&format!("- {span} {}", e.title));
        if !e.location.is_empty() {
            out.push_str(&format!(" @ {}", e.location));
        }
        if with_ids {
            out.push_str(&format!(" [id: {}]", e.key));
        }
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("Nenhum evento no período.");
    }
    out.trim_end().to_string()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_get_the_local_offset() {
        let s = local_to_rfc3339("2026-11-20T15:30").unwrap();
        assert!(s.starts_with("2026-11-20T15:30:00"));
        assert!(s.len() > 19, "an offset is attached: {s}");
        assert!(local_to_rfc3339("amanhã às 3").is_err());
    }

    #[test]
    fn all_day_events_end_the_next_day() {
        let (s, e) = when("2026-11-20", None, None, true).unwrap();
        assert_eq!(s["date"], "2026-11-20");
        assert_eq!(e["date"], "2026-11-21");
    }

    #[test]
    fn timed_events_default_to_one_hour_and_reject_backwards_ranges() {
        let (s, e) = when("2026-11-20T15:00", None, None, false).unwrap();
        let a = chrono::DateTime::parse_from_rfc3339(s["dateTime"].as_str().unwrap()).unwrap();
        let b = chrono::DateTime::parse_from_rfc3339(e["dateTime"].as_str().unwrap()).unwrap();
        assert_eq!((b - a).num_minutes(), 60);
        assert!(when("2026-11-20T15:00", Some("2026-11-20T14:00"), None, false).is_err());
    }

    #[test]
    fn popup_reminders_come_from_overrides_or_the_calendar_defaults() {
        let over = json!({ "useDefault": false, "overrides": [{ "method": "popup", "minutes": 10 }, { "method": "email", "minutes": 60 }] });
        assert_eq!(popup_minutes(&over, &[30]), vec![10]);
        assert_eq!(popup_minutes(&json!({ "useDefault": true }), &[30]), vec![30]);
        assert_eq!(popup_minutes(&Value::Null, &[]), Vec::<i64>::new());
        assert_eq!(reminder_body(0)["overrides"][0]["minutes"], 0);
    }

    #[test]
    fn agenda_text_groups_by_day() {
        let ev = |start: &str, end: &str, title: &str, all_day: bool| CalEvent {
            key: "c|e".into(), title: title.into(), start: start.into(), end: end.into(), all_day,
            location: String::new(), description: String::new(), calendar: "Agenda".into(),
            color: String::new(), link: String::new(), meet: String::new(), reminders: Vec::new(),
        };
        let s = local_to_rfc3339("2026-11-20T15:30").unwrap();
        let e = local_to_rfc3339("2026-11-20T16:30").unwrap();
        let text = describe(&[ev("2026-11-20", "2026-11-21", "Feriado", true), ev(&s, &e, "Reunião", false)], false);
        assert!(text.starts_with("sexta-feira 20/11"), "{text}");
        assert!(text.contains("- dia todo Feriado"));
        assert!(text.contains("- 15:30–16:30 Reunião"));
        assert_eq!(describe(&[], false), "Nenhum evento no período.");
    }

    #[test]
    fn the_project_number_comes_from_the_client_id() {
        assert_eq!(project_number("206533512741-abc123.apps.googleusercontent.com"), "206533512741");
        assert_eq!(project_number("not-a-google-id"), "");
    }

    #[test]
    fn event_keys_survive_odd_calendar_ids() {
        assert_eq!(split_key("a@group.calendar.google.com|abc").unwrap().0, "a@group.calendar.google.com");
        assert!(enc("a@group.calendar.google.com").contains('@') || enc("a@group.calendar.google.com").contains("%40"));
        assert!(split_key("semid").is_err());
    }
}
