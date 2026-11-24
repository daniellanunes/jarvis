//! Spotify Web API (Premium): login with PKCE on a loopback port, search, play
//! the exact track/album/playlist/artist asked for on the desktop app (Spotify
//! Connect), player state and controls for the in-Jarvis Spotify panel.
//!
//! Setup, once: create an app at developer.spotify.com, add the redirect URI
//! `http://127.0.0.1:43821/callback`, and paste its Client ID in
//! Ajustes → Jarvis (or set `SPOTIFY_CLIENT_ID`). The refresh token lives in
//! the OS keychain (`spotify-refresh-token`); access tokens only in memory.

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::secrets;

pub const REDIRECT_PORT: u16 = 43821;
pub const REDIRECT_URI: &str = "http://127.0.0.1:43821/callback";
const API: &str = "https://api.spotify.com/v1";
const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const REFRESH_KEY: &str = "spotify-refresh-token";
const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing \
playlist-read-private playlist-read-collaborative user-library-read user-read-recently-played user-top-read";

struct Token {
    access: String,
    expires: Instant,
}

static TOKEN: LazyLock<Mutex<Option<Token>>> = LazyLock::new(|| Mutex::new(None));

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())
}

pub fn client_id() -> Option<String> {
    crate::jarvis_config::spotify_client_id()
}

/// True once the user logged in (a refresh token is stored) and a Client ID is set.
pub fn connected() -> bool {
    client_id().is_some() && secrets::present(REFRESH_KEY)
}

// ── PKCE login ────────────────────────────────────────────────────────────────

fn b64url(bytes: &[u8]) -> String {
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
    let q = path.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| {
            reqwest::Url::parse(&format!("http://x/?v={v}"))
                .ok()
                .and_then(|u| u.query_pairs().next().map(|(_, v)| v.to_string()))
                .unwrap_or_else(|| v.to_string())
        })
    })
}

fn page(title: &str, msg: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
<body style=\"font:15px system-ui;background:#0e0f11;color:#e7e7ea;display:grid;place-items:center;height:100vh;margin:0\">\
<div style=\"max-width:420px;text-align:center\"><h2 style=\"font-weight:600\">{title}</h2><p style=\"color:#9a9aa3\">{msg}</p></div>"
    )
}

async fn exchange(form: &[(&str, &str)]) -> Result<Value, String> {
    let res = client()?
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Spotify: falha de conexão ({e})"))?;
    let status = res.status();
    let body: Value = res.json().await.map_err(|e| format!("Spotify: resposta inválida ({e})"))?;
    if !status.is_success() {
        let msg = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("erro");
        return Err(format!("Spotify ({status}): {msg}"));
    }
    Ok(body)
}

fn store_tokens(body: &Value) -> Result<String, String> {
    let access = body["access_token"].as_str().ok_or("Spotify não devolveu access_token")?.to_string();
    let secs = body["expires_in"].as_u64().unwrap_or(3600);
    if let Some(refresh) = body["refresh_token"].as_str() {
        secrets::set(REFRESH_KEY, refresh)?;
    }
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Token {
        access: access.clone(),
        expires: Instant::now() + Duration::from_secs(secs.saturating_sub(60)),
    });
    Ok(access)
}

/// Opens the Spotify consent page and waits (up to 3 min) for the redirect.
#[tauri::command]
pub async fn spotify_login() -> Result<String, String> {
    let cid = client_id().ok_or(
        "Cole o Client ID do seu app do Spotify em Ajustes → Jarvis → Spotify antes de conectar.",
    )?;
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| format!("Porta {REDIRECT_PORT} ocupada ({e}). Feche outro login do Spotify e tente de novo."))?;

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
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("state", &state)
        .append_pair("scope", SCOPES);
    crate::jarvis_pc::jarvis_open_target(url.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Tempo esgotado esperando o login do Spotify.".into());
        }
        let (mut stream, _) = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| "Tempo esgotado esperando o login do Spotify.".to_string())?
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        if !path.starts_with("/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let respond = |ok: bool, msg: &str| {
            let html = page(if ok { "Spotify conectado" } else { "Login não concluído" }, msg);
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            )
        };
        if query_param(&path, "state").as_deref() != Some(state.as_str()) {
            let _ = stream.write_all(respond(false, "State inválido. Tente conectar de novo.").as_bytes()).await;
            return Err("Login do Spotify recusado (state inválido).".into());
        }
        if let Some(err) = query_param(&path, "error") {
            let _ = stream.write_all(respond(false, &err).as_bytes()).await;
            return Err(format!("Login do Spotify cancelado: {err}"));
        }
        let code = query_param(&path, "code").ok_or("Spotify não devolveu o código.")?;
        let body = exchange(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", &cid),
            ("code_verifier", &verifier),
        ])
        .await;
        return match body.and_then(|b| store_tokens(&b)) {
            Ok(_) => {
                let _ = stream
                    .write_all(respond(true, "Pode fechar esta aba e voltar para o Jarvis.").as_bytes())
                    .await;
                Ok("Spotify conectado.".into())
            }
            Err(e) => {
                let _ = stream.write_all(respond(false, &e).as_bytes()).await;
                Err(e)
            }
        };
    }
}

#[tauri::command]
pub fn spotify_logout() -> Result<(), String> {
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    secrets::clear(REFRESH_KEY)
}

async fn access_token(force_refresh: bool) -> Result<String, String> {
    if !force_refresh {
        if let Some(t) = TOKEN.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if Instant::now() < t.expires {
                return Ok(t.access.clone());
            }
        }
    }
    let cid = client_id().ok_or("Spotify sem Client ID configurado.")?;
    let refresh = secrets::get(REFRESH_KEY).ok_or("Spotify não conectado. Conecte em Ajustes → Jarvis → Spotify.")?;
    let body = exchange(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &cid)]).await?;
    store_tokens(&body)
}

// ── API calls ────────────────────────────────────────────────────────────────

async fn call(method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Option<Value>, String> {
    let mut refreshed = false;
    loop {
        let token = access_token(refreshed).await?;
        let mut req = client()?.request(method.clone(), format!("{API}{path}")).bearer_auth(&token);
        req = match &body {
            Some(b) => req.json(b),
            None if method != reqwest::Method::GET => req.header("content-length", "0"),
            None => req,
        };
        let res = req.send().await.map_err(|e| format!("Spotify: falha de conexão ({e})"))?;
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
        let msg = v["error"]["message"].as_str().unwrap_or(text.as_str()).to_string();
        let reason = v["error"]["reason"].as_str().unwrap_or("");
        return Err(match (status, reason) {
            (404, _) | (_, "NO_ACTIVE_DEVICE") => "NO_DEVICE".into(),
            (403, "PREMIUM_REQUIRED") => "Essa ação exige Spotify Premium.".into(),
            (429, _) => "Spotify limitou as requisições; tente em alguns segundos.".into(),
            _ => format!("Spotify ({status}): {msg}"),
        });
    }
}

async fn get(path: &str) -> Result<Option<Value>, String> {
    call(reqwest::Method::GET, path, None).await
}

fn enc(s: &str) -> String {
    let mut u = reqwest::Url::parse("http://x/").unwrap();
    u.query_pairs_mut().append_pair("q", s);
    u.query().unwrap_or("q=").trim_start_matches("q=").to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub kind: String,
    pub uri: String,
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub image: String,
    pub duration_ms: u64,
}

fn image_of(v: &Value) -> String {
    let imgs = v["images"].as_array().or_else(|| v["album"]["images"].as_array());
    imgs.and_then(|a| a.iter().rev().find(|i| i["width"].as_u64().unwrap_or(0) >= 160).or(a.first()))
        .and_then(|i| i["url"].as_str())
        .unwrap_or("")
        .to_string()
}

fn item_of(kind: &str, v: &Value) -> Option<Item> {
    let uri = v["uri"].as_str()?.to_string();
    let artists = v["artists"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let subtitle = match kind {
        "track" => format!("{artists} · {}", v["album"]["name"].as_str().unwrap_or("")),
        "album" => artists,
        "playlist" => format!(
            "{} · {} músicas",
            v["owner"]["display_name"].as_str().unwrap_or("Playlist"),
            v["tracks"]["total"].as_u64().or(v["items"]["total"].as_u64()).unwrap_or(0)
        ),
        "artist" => "Artista".into(),
        _ => String::new(),
    };
    Some(Item {
        kind: kind.into(),
        uri,
        id: v["id"].as_str().unwrap_or("").into(),
        name: v["name"].as_str().unwrap_or("").into(),
        subtitle,
        image: image_of(v),
        duration_ms: v["duration_ms"].as_u64().unwrap_or(0),
    })
}

pub async fn search(query: &str, kinds: &str, limit: u32) -> Result<Vec<Item>, String> {
    let path = format!("/search?q={}&type={kinds}&limit={limit}", enc(query));
    let v = get(&path).await?.unwrap_or(Value::Null);
    let mut out = Vec::new();
    for kind in kinds.split(',') {
        if let Some(items) = v[format!("{kind}s")]["items"].as_array() {
            out.extend(items.iter().filter(|x| !x.is_null()).filter_map(|x| item_of(kind, x)));
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn spotify_search(query: String, kinds: Option<String>) -> Result<Vec<Item>, String> {
    search(&query, kinds.as_deref().unwrap_or("track,album,playlist,artist"), 8).await
}

#[tauri::command]
pub async fn spotify_my_playlists() -> Result<Vec<Item>, String> {
    let v = get("/me/playlists?limit=50").await?.unwrap_or(Value::Null);
    Ok(v["items"]
        .as_array()
        .map(|a| a.iter().filter(|x| !x.is_null()).filter_map(|x| item_of("playlist", x)).collect())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn spotify_collection_tracks(uri: String) -> Result<Vec<Item>, String> {
    let mut parts = uri.splitn(3, ':');
    let (_, kind, id) = (parts.next(), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    match kind {
        "playlist" => {
            // Spotify renamed playlist "tracks" to "items" (and each entry's
            // "track" to "item") for development-mode apps: ask the new
            // endpoint first, the old one if this account still has it.
            let v = match get(&format!("/playlists/{id}/items?limit=100")).await {
                Ok(v) => v,
                Err(_) => get(&format!("/playlists/{id}/tracks?limit=100")).await?,
            }
            .unwrap_or(Value::Null);
            Ok(playlist_entries(&v))
        }
        "album" => {
            // The album itself, so every track gets its cover and album name.
            let album = get(&format!("/albums/{id}")).await?.unwrap_or(Value::Null);
            let list = album["tracks"]["items"].as_array().cloned().unwrap_or_default();
            Ok(list
                .iter()
                .filter_map(|t| {
                    let mut t = t.clone();
                    t["album"] = json!({ "name": album["name"], "images": album["images"] });
                    item_of("track", &t)
                })
                .collect())
        }
        "artist" => {
            let v = get(&format!("/artists/{id}/top-tracks")).await?.unwrap_or(Value::Null);
            Ok(v["tracks"].as_array().map(|a| a.iter().filter_map(|t| item_of("track", t)).collect()).unwrap_or_default())
        }
        // "Músicas Curtidas": the user's saved tracks.
        "collection" => {
            let v = get("/me/tracks?limit=50").await?.unwrap_or(Value::Null);
            Ok(playlist_entries(&v))
        }
        _ => Err("Coleção desconhecida".into()),
    }
}

/// Tracks out of a playlist / saved-tracks page, old ("track") or new ("item") shape.
fn playlist_entries(v: &Value) -> Vec<Item> {
    v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let t = if x["item"].is_object() { &x["item"] } else { &x["track"] };
                    // Podcasts and local files have no playable track URI here.
                    (t["type"].as_str().unwrap_or("track") == "track").then(|| item_of("track", t)).flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Recently played tracks (newest first, without repeats).
#[tauri::command]
pub async fn spotify_recent() -> Result<Vec<Item>, String> {
    let v = get("/me/player/recently-played?limit=40").await?.unwrap_or(Value::Null);
    let mut seen = std::collections::HashSet::new();
    Ok(v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| item_of("track", &x["track"]))
                .filter(|i| seen.insert(i.uri.clone()))
                .collect()
        })
        .unwrap_or_default())
}

/// How many songs the user has liked (for the library row).
#[tauri::command]
pub async fn spotify_liked_count() -> Result<u64, String> {
    let v = get("/me/tracks?limit=1").await?.unwrap_or(Value::Null);
    Ok(v["total"].as_u64().unwrap_or(0))
}

/// Plays a list of tracks starting at `offset` (liked songs, recents…).
#[tauri::command]
pub async fn spotify_play_list(uris: Vec<String>, offset: usize) -> Result<(), String> {
    if uris.is_empty() {
        return Err("Lista vazia".into());
    }
    let device = ensure_device().await?;
    let uris: Vec<String> = uris.into_iter().take(100).collect();
    let offset = offset.min(uris.len() - 1);
    call(
        reqwest::Method::PUT,
        &format!("/me/player/play?device_id={device}"),
        Some(json!({ "uris": uris, "offset": { "position": offset } })),
    )
    .await
    .map(|_| ())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub volume: Option<u64>,
}

pub async fn devices() -> Result<Vec<Device>, String> {
    let v = get("/me/player/devices").await?.unwrap_or(Value::Null);
    Ok(v["devices"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| Device {
                    id: d["id"].as_str().unwrap_or("").into(),
                    name: d["name"].as_str().unwrap_or("").into(),
                    kind: d["type"].as_str().unwrap_or("").into(),
                    active: d["is_active"].as_bool().unwrap_or(false),
                    volume: d["volume_percent"].as_u64(),
                })
                .filter(|d| !d.id.is_empty())
                .collect()
        })
        .unwrap_or_default())
}

#[tauri::command]
pub async fn spotify_devices() -> Result<Vec<Device>, String> {
    devices().await
}

/// Spotify closed: start it now, so it boots while the search runs.
async fn wake_app() {
    if devices().await.is_ok_and(|d| d.is_empty()) {
        crate::log::line("spotify: app fechado; abrindo antes de tocar");
        let _ = crate::jarvis_pc::jarvis_open_target("spotify".into());
    }
}

/// The device to play on: the active one, else this PC's Spotify app (started
/// if needed). Right after the PC starts, or while Spotify updates or signs
/// in, it can take 15-20 s to register with Spotify Connect: wait up to 30 s.
async fn ensure_device() -> Result<String, String> {
    let host = std::env::var("COMPUTERNAME").unwrap_or_default().to_lowercase();
    let pick = |list: &[Device]| -> Option<String> {
        list.iter()
            .find(|d| d.active)
            .or_else(|| list.iter().find(|d| d.kind == "Computer" && !host.is_empty() && d.name.to_lowercase().contains(&host)))
            .or_else(|| list.iter().find(|d| d.kind == "Computer"))
            .or_else(|| list.first())
            .map(|d| d.id.clone())
    };
    if let Some(id) = pick(&devices().await?) {
        return Ok(id);
    }
    let _ = crate::jarvis_pc::jarvis_open_target("spotify".into());
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(id) = pick(&devices().await.unwrap_or_default()) {
            crate::log::line(format!("spotify: pronto em {:.1}s", started.elapsed().as_secs_f32()));
            return Ok(id);
        }
    }
    crate::log::line("spotify: o app não apareceu no Spotify Connect em 30 s");
    Err("Abri o Spotify, mas ele não ficou pronto em 30 segundos. Confira se ele entrou na sua conta e peça de novo.".into())
}

/// Plays a URI: a track (optionally inside its album/playlist context) or a
/// collection (album, playlist, artist). Retries once on a fresh device.
pub async fn play_uri(uri: &str, context: Option<&str>) -> Result<(), String> {
    let body = if uri.starts_with("spotify:track:") {
        match context.filter(|c| !c.is_empty() && !c.starts_with("spotify:artist:")) {
            Some(ctx) => json!({ "context_uri": ctx, "offset": { "uri": uri } }),
            None => json!({ "uris": [uri] }),
        }
    } else {
        json!({ "context_uri": uri })
    };
    let device = ensure_device().await?;
    let path = format!("/me/player/play?device_id={device}");
    match call(reqwest::Method::PUT, &path, Some(body.clone())).await {
        Ok(_) => Ok(()),
        Err(e) if e == "NO_DEVICE" => {
            // The app was asleep or still starting: wake it on this device,
            // then play — a few times, it can take some seconds to accept.
            for attempt in 0..8 {
                let _ = call(reqwest::Method::PUT, "/me/player", Some(json!({ "device_ids": [device], "play": false }))).await;
                tokio::time::sleep(Duration::from_millis(if attempt == 0 { 600 } else { 1200 })).await;
                match call(reqwest::Method::PUT, &path, Some(body.clone())).await {
                    Ok(_) => return Ok(()),
                    Err(e) if e == "NO_DEVICE" => continue,
                    Err(e) => return Err(e),
                }
            }
            crate::log::line("spotify: o app não aceitou tocar (NO_DEVICE)");
            Err("O Spotify abriu, mas não aceitou tocar ainda. Peça de novo em alguns segundos.".into())
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn spotify_play_uri(uri: String, context: Option<String>) -> Result<(), String> {
    play_uri(&uri, context.as_deref()).await
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' => 'e',
            'í' | 'ì' => 'i',
            'ó' | 'ô' | 'õ' | 'ò' => 'o',
            'ú' | 'ü' | 'ù' => 'u',
            'ç' => 'c',
            c if c.is_alphanumeric() || c == ' ' => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Guesses the kind from words in the request ("álbum X", "playlist X"…) and
/// strips them from the query.
pub fn classify(query: &str, hint: Option<&str>) -> (String, String) {
    let n = normalize(query);
    let mut kind = hint.map(str::to_string).filter(|h| ["track", "album", "playlist", "artist"].contains(&h.as_str()));
    let mut words: Vec<&str> = n.split(' ').collect();
    let markers: &[(&str, &str)] = &[
        ("album", "album"),
        ("disco", "album"),
        ("playlist", "playlist"),
        ("artista", "artist"),
        ("musica", "track"),
        ("faixa", "track"),
        ("cancao", "track"),
    ];
    words.retain(|w| {
        if let Some((_, k)) = markers.iter().find(|(m, _)| m == w) {
            if kind.is_none() {
                kind = Some(k.to_string());
            }
            false
        } else {
            !matches!(
                *w,
                "o" | "a" | "as" | "os" | "do" | "da" | "de" | "no" | "na" | "spotify" | "minha" | "meu" | "musicas"
            )
        }
    });
    if n.starts_with("musicas de ") || n.starts_with("as musicas de ") || n.contains("mais tocadas de") {
        kind = Some("artist".into());
    }
    (kind.unwrap_or_else(|| "track".into()), words.join(" "))
}

fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    if a == b {
        return 1.0;
    }
    let aw: Vec<&str> = a.split(' ').collect();
    let hits = aw.iter().filter(|w| b.contains(*w)).count();
    hits as f32 / aw.len().max(1) as f32 * 0.9
}

/// "toca X": resolves X to the best match and plays it. For playlists the
/// user's own playlists are checked first ("toca minha playlist de treino").
pub async fn play_query(query: &str, hint: Option<&str>) -> Result<String, String> {
    wake_app().await;
    let (kind, q) = classify(query, hint);
    if q.trim().is_empty() {
        call(reqwest::Method::PUT, &format!("/me/player/play?device_id={}", ensure_device().await?), None).await?;
        return Ok("Reprodução retomada no Spotify.".into());
    }

    let mut best: Option<Item> = None;
    if kind == "playlist" {
        if let Ok(mine) = spotify_my_playlists().await {
            best = mine
                .into_iter()
                .map(|p| (similarity(&q, &p.name), p))
                .filter(|(s, _)| *s >= 0.6)
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p)| p);
        }
    }
    if best.is_none() {
        let found = search(&q, &kind, 5).await?;
        // Spotify ranks by popularity: prefer an exact title match when there is one.
        best = found
            .iter()
            .find(|i| normalize(&i.name) == normalize(&q) || similarity(&i.name, &q) >= 0.99)
            .cloned()
            .or_else(|| found.into_iter().next());
    }
    let item = best.ok_or_else(|| format!("Não encontrei \"{q}\" no Spotify."))?;
    play_uri(&item.uri, None).await?;

    // Confirm what is actually playing, so the reply is never a guess.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let now = state().await.ok().flatten();
    let label = match item.kind.as_str() {
        "album" => "álbum",
        "playlist" => "playlist",
        "artist" => "artista",
        _ => "música",
    };
    Ok(match now {
        Some(s) if s.playing => format!("Tocando {label} {} — agora: {} · {}", item.name, s.title, s.artist),
        _ => format!("Pedi ao Spotify para tocar {label} {} ({}).", item.name, item.subtitle),
    })
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub image: String,
    pub uri: String,
    pub context_uri: String,
    pub progress_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub repeat: String,
    pub volume: Option<u64>,
    pub device: String,
}

pub async fn state() -> Result<Option<PlayerState>, String> {
    let Some(v) = get("/me/player?additional_types=track,episode").await? else {
        return Ok(None);
    };
    let item = &v["item"];
    Ok(Some(PlayerState {
        playing: v["is_playing"].as_bool().unwrap_or(false),
        title: item["name"].as_str().unwrap_or("").into(),
        artist: item["artists"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
            .or_else(|| item["show"]["name"].as_str().map(str::to_string))
            .unwrap_or_default(),
        album: item["album"]["name"].as_str().unwrap_or("").into(),
        image: image_of(item),
        uri: item["uri"].as_str().unwrap_or("").into(),
        context_uri: v["context"]["uri"].as_str().unwrap_or("").into(),
        progress_ms: v["progress_ms"].as_u64().unwrap_or(0),
        duration_ms: item["duration_ms"].as_u64().unwrap_or(0),
        shuffle: v["shuffle_state"].as_bool().unwrap_or(false),
        repeat: v["repeat_state"].as_str().unwrap_or("off").into(),
        volume: v["device"]["volume_percent"].as_u64(),
        device: v["device"]["name"].as_str().unwrap_or("").into(),
    }))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub client_id_set: bool,
    pub connected: bool,
    pub redirect_uri: String,
}

#[tauri::command]
pub fn spotify_api_status() -> ApiStatus {
    ApiStatus {
        client_id_set: client_id().is_some(),
        connected: connected(),
        redirect_uri: REDIRECT_URI.into(),
    }
}

#[tauri::command]
pub async fn spotify_player_state() -> Result<Option<PlayerState>, String> {
    state().await
}

/// play | pause | next | previous | seek (ms) | volume (0-100) | shuffle (0/1)
/// | repeat (0 off, 1 context, 2 track) | queue (uri in `uri`) | transfer (device id in `uri`).
pub async fn control(action: &str, value: Option<f64>, uri: Option<&str>) -> Result<(), String> {
    use reqwest::Method as M;
    let v = value.unwrap_or(0.0);
    let r = match action {
        "play" => call(M::PUT, "/me/player/play", None).await,
        "pause" => call(M::PUT, "/me/player/pause", None).await,
        "next" => call(M::POST, "/me/player/next", None).await,
        "previous" | "prev" => call(M::POST, "/me/player/previous", None).await,
        "seek" => call(M::PUT, &format!("/me/player/seek?position_ms={}", v.max(0.0) as u64), None).await,
        "volume" => call(M::PUT, &format!("/me/player/volume?volume_percent={}", v.clamp(0.0, 100.0) as u64), None).await,
        "shuffle" => call(M::PUT, &format!("/me/player/shuffle?state={}", v > 0.0), None).await,
        "repeat" => {
            let mode = match v as i64 { 1 => "context", 2 => "track", _ => "off" };
            call(M::PUT, &format!("/me/player/repeat?state={mode}"), None).await
        }
        "queue" => call(M::POST, &format!("/me/player/queue?uri={}", enc(uri.unwrap_or(""))), None).await,
        "transfer" => {
            call(M::PUT, "/me/player", Some(json!({ "device_ids": [uri.unwrap_or("")], "play": true }))).await
        }
        other => return Err(format!("Ação desconhecida: {other}")),
    };
    match r {
        Ok(_) => Ok(()),
        Err(e) if e == "NO_DEVICE" && action == "play" => {
            let device = ensure_device().await?;
            call(M::PUT, &format!("/me/player/play?device_id={device}"), None).await.map(|_| ())
        }
        Err(e) if e == "NO_DEVICE" => Err("Nenhum Spotify tocando agora.".into()),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn spotify_api_control(action: String, value: Option<f64>, uri: Option<String>) -> Result<(), String> {
    control(&action, value, uri.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_classified_and_cleaned() {
        assert_eq!(classify("o álbum Dawn FM do The Weeknd", None), ("album".into(), "dawn fm the weeknd".into()));
        assert_eq!(classify("minha playlist de treino", None), ("playlist".into(), "treino".into()));
        assert_eq!(classify("Blinding Lights", None), ("track".into(), "blinding lights".into()));
        assert_eq!(classify("músicas de Djavan", None).0, "artist");
        assert_eq!(classify("Dawn FM", Some("album")).0, "album");
    }

    #[test]
    fn exact_titles_win_over_partial_ones() {
        assert_eq!(similarity("Treino", "treino"), 1.0);
        assert!(similarity("treino pesado", "Treino") < similarity("treino", "Treino"));
    }

    #[test]
    fn playlist_entries_read_both_api_shapes() {
        let track = |n: &str| json!({ "type": "track", "uri": format!("spotify:track:{n}"), "id": n, "name": n, "artists": [], "album": { "name": "A", "images": [] }, "duration_ms": 1000 });
        let old = json!({ "items": [{ "track": track("a") }, { "track": { "type": "episode", "uri": "spotify:episode:x" } }] });
        let new = json!({ "items": [{ "item": track("b") }, { "item": track("c") }] });
        assert_eq!(playlist_entries(&old).len(), 1);
        assert_eq!(playlist_entries(&new).iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    }

    #[test]
    fn base64url_has_no_padding() {
        assert_eq!(b64url(b"hello"), "aGVsbG8");
        assert!(!b64url(&[255u8; 32]).contains(['+', '/', '=']));
    }

    #[test]
    fn query_params_are_decoded() {
        assert_eq!(query_param("/callback?code=a%2Bb&state=x", "code").as_deref(), Some("a+b"));
        assert_eq!(query_param("/callback?state=x", "code"), None);
    }
}

#[cfg(all(test, windows))]
mod live_device {
    /// How long a closed Spotify takes to show up as a device. By hand:
    /// cargo test --lib spotify_api::live_device -- --ignored --nocapture
    #[test]
    #[ignore]
    fn closed_spotify_shows_up_as_a_device() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        crate::jarvis_config::apply_settings(&crate::settings::load());
        rt.block_on(async {
            println!("conectado: {}", super::connected());
            println!("antes: {:?}", super::devices().await.map(|d| d.iter().map(|x| x.name.clone()).collect::<Vec<_>>()));
            let t = std::time::Instant::now();
            let _ = crate::jarvis_pc::jarvis_open_target("spotify".into());
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let d = super::devices().await;
                if let Ok(list) = &d {
                    if !list.is_empty() {
                        println!("apareceu em {:.1}s: {:?}", t.elapsed().as_secs_f32(), list.iter().map(|x| (&x.name, &x.kind, x.active)).collect::<Vec<_>>());
                        return;
                    }
                } else {
                    println!("erro: {d:?}");
                }
            }
            println!("não apareceu em {:.1}s", t.elapsed().as_secs_f32());
        });
    }
}

#[cfg(all(test, windows))]
mod live_play {
    /// The whole "toca X" with Spotify closed. By hand:
    /// cargo test --lib spotify_api::live_play -- --ignored --nocapture
    #[test]
    #[ignore]
    fn plays_with_spotify_closed() {
        crate::jarvis_config::apply_settings(&crate::settings::load());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let t = std::time::Instant::now();
            let r = super::play_query("The Kill Thirty Seconds to Mars", None).await;
            println!("resultado em {:.1}s: {r:?}", t.elapsed().as_secs_f32());
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            println!("estado: {:?}", super::state().await.map(|s| s.map(|s| (s.playing, s.title, s.device))));
            let _ = super::control("pause", None, None).await;
        });
    }
}
