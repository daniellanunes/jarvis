//! Gmail through the same Google login as the Agenda. Nothing here runs on
//! its own: the brain calls it only when the user asks about e-mail, and the
//! E-mail tab only when opened. Sending is a draft first, then Permitir.

use serde::Serialize;
use serde_json::{json, Value};

use crate::google_calendar as google;

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MailItem {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub date: String,
    pub snippet: String,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MailFull {
    #[serde(flatten)]
    pub item: MailItem,
    pub to: String,
    pub body: String,
    pub message_id: String,
}

async fn call(method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
    if !google::connected() {
        return Err("Conta Google não conectada. Conecte na aba Agenda (o Gmail usa o mesmo login).".into());
    }
    let mut refreshed = false;
    loop {
        let token = google::access_token(refreshed).await?;
        let mut req = google::client()?.request(method.clone(), format!("{API}{path}")).bearer_auth(&token);
        if let Some(b) = &body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|e| format!("Gmail: falha de conexão ({e})"))?;
        let status = res.status().as_u16();
        if status == 401 && !refreshed {
            refreshed = true;
            continue;
        }
        let text = res.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(v);
        }
        let msg = v["error"]["message"].as_str().unwrap_or(text.as_str()).to_string();
        if status == 403 && (msg.contains("has not been used") || msg.contains("is disabled")) {
            return Err("A Gmail API não está ativada no seu projeto do Google Cloud. Ative em console.cloud.google.com/apis/library/gmail.googleapis.com e tente de novo.".into());
        }
        if status == 403 && (msg.to_lowercase().contains("insufficient") || msg.contains("scope")) {
            return Err("O Jarvis ainda não tem permissão para o Gmail. Na aba Agenda, clique no seu e-mail embaixo do título → Reconectar conta, e marque o acesso ao Gmail na tela do Google.".into());
        }
        return Err(format!("Gmail ({status}): {msg}"));
    }
}

fn header(msg: &Value, name: &str) -> String {
    msg["payload"]["headers"]
        .as_array()
        .and_then(|hs| hs.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))))
        .and_then(|h| h["value"].as_str())
        .unwrap_or("")
        .to_string()
}

fn item_of(msg: &Value) -> MailItem {
    MailItem {
        id: msg["id"].as_str().unwrap_or("").into(),
        thread_id: msg["threadId"].as_str().unwrap_or("").into(),
        from: header(msg, "From"),
        subject: header(msg, "Subject"),
        date: header(msg, "Date"),
        snippet: html_unescape(msg["snippet"].as_str().unwrap_or("")),
        unread: msg["labelIds"].as_array().is_some_and(|l| l.iter().any(|x| x == "UNREAD")),
    }
}

/// Base64url (padding optional) → bytes.
pub fn b64url_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&amp;", "&")
}

/// Rough HTML → text for mails without a plain part.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut skip = false;
    let lower = html.to_lowercase();
    let bytes: Vec<char> = html.chars().collect();
    let lchars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '<' {
            in_tag = true;
            let rest: String = lchars[i..(i + 8).min(lchars.len())].iter().collect();
            if rest.starts_with("<style") || rest.starts_with("<script") {
                skip = true;
            } else if rest.starts_with("</style") || rest.starts_with("</script") {
                skip = false;
            } else if rest.starts_with("<br") || rest.starts_with("<p") || rest.starts_with("</p") || rest.starts_with("<div") || rest.starts_with("<tr") || rest.starts_with("<li") {
                out.push('\n');
            }
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag && !skip {
            out.push(c);
        }
        i += 1;
    }
    let text = html_unescape(&out);
    let mut lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

fn body_of(part: &Value) -> (Option<String>, Option<String>) {
    let mime = part["mimeType"].as_str().unwrap_or("");
    let data = part["body"]["data"].as_str().map(|d| String::from_utf8_lossy(&b64url_decode(d)).to_string());
    let (mut plain, mut html) = (None, None);
    if mime == "text/plain" {
        plain = data;
    } else if mime == "text/html" {
        html = data;
    }
    if let Some(parts) = part["parts"].as_array() {
        for p in parts {
            let (pp, ph) = body_of(p);
            plain = plain.or(pp);
            html = html.or(ph);
        }
    }
    (plain, html)
}

/// Messages matching a Gmail search ("is:unread", "from:ana", "newer_than:2d").
pub async fn list(query: &str, max: u32) -> Result<Vec<MailItem>, String> {
    let mut url = reqwest::Url::parse(&format!("{API}/messages")).unwrap();
    url.query_pairs_mut()
        .append_pair("maxResults", &max.clamp(1, 25).to_string())
        .append_pair("q", if query.trim().is_empty() { "in:inbox" } else { query.trim() });
    let path = format!("/messages?{}", url.query().unwrap_or(""));
    let v = call(reqwest::Method::GET, &path, None).await?;
    let ids: Vec<String> = v["messages"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default();
    let fetches = ids.iter().map(|id| async move {
        let path = format!("/messages/{id}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date");
        call(reqwest::Method::GET, &path, None).await
    });
    let mut out = Vec::new();
    for r in futures_util::future::join_all(fetches).await {
        out.push(item_of(&r?));
    }
    Ok(out)
}

pub async fn read(id: &str) -> Result<MailFull, String> {
    let v = call(reqwest::Method::GET, &format!("/messages/{id}?format=full"), None).await?;
    let (plain, html) = body_of(&v["payload"]);
    let mut body = plain.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).or_else(|| html.map(|h| html_to_text(&h))).unwrap_or_default();
    if body.chars().count() > 12000 {
        body = body.chars().take(12000).collect::<String>() + "\n[…]";
    }
    Ok(MailFull { item: item_of(&v), to: header(&v, "To"), message_id: header(&v, "Message-ID"), body })
}

/// The address inside "Ana <ana@x.com>".
fn address(from: &str) -> String {
    match (from.rfind('<'), from.rfind('>')) {
        (Some(a), Some(b)) if b > a => from[a + 1..b].to_string(),
        _ => from.trim().to_string(),
    }
}

/// RFC 2047 for non-ASCII subjects.
fn encode_subject(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        let mut b64 = google::b64url(s.as_bytes()).replace('-', "+").replace('_', "/");
        while b64.len() % 4 != 0 {
            b64.push('=');
        }
        format!("=?UTF-8?B?{b64}?=")
    }
}

pub fn build_raw(to: &str, subject: &str, body: &str, in_reply_to: Option<&str>) -> String {
    let mut msg = format!(
        "To: {to}\r\nSubject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n",
        encode_subject(subject)
    );
    if let Some(r) = in_reply_to.filter(|r| !r.is_empty()) {
        msg.push_str(&format!("In-Reply-To: {r}\r\nReferences: {r}\r\n"));
    }
    msg.push_str("\r\n");
    msg.push_str(&body.replace("\r\n", "\n").replace('\n', "\r\n"));
    google::b64url(msg.as_bytes())
}

pub struct Draft {
    pub id: String,
    pub to: String,
    pub subject: String,
}

/// A draft (new, or a reply to `reply_to` — a message id). Never sends.
pub async fn draft(to: Option<&str>, subject: Option<&str>, body: &str, reply_to: Option<&str>) -> Result<Draft, String> {
    let (mut to, mut subject, mut thread, mut in_reply) = (to.unwrap_or("").to_string(), subject.unwrap_or("").to_string(), None, None);
    if let Some(id) = reply_to.filter(|s| !s.is_empty()) {
        let orig = read(id).await?;
        if to.is_empty() {
            to = address(&orig.item.from);
        }
        if subject.is_empty() {
            subject = if orig.item.subject.to_lowercase().starts_with("re:") { orig.item.subject.clone() } else { format!("Re: {}", orig.item.subject) };
        }
        thread = Some(orig.item.thread_id.clone());
        in_reply = Some(orig.message_id.clone());
    }
    if to.trim().is_empty() {
        return Err("Para quem é o e-mail? Falta o destinatário.".into());
    }
    let mut message = json!({ "raw": build_raw(&to, &subject, body, in_reply.as_deref()) });
    if let Some(t) = thread {
        message["threadId"] = json!(t);
    }
    let v = call(reqwest::Method::POST, "/drafts", Some(json!({ "message": message }))).await?;
    Ok(Draft { id: v["id"].as_str().unwrap_or("").into(), to, subject })
}

/// Sends a draft — only from the Permitir button.
pub async fn send_draft(draft_id: &str) -> Result<String, String> {
    call(reqwest::Method::POST, "/drafts/send", Some(json!({ "id": draft_id }))).await?;
    Ok("E-mail enviado.".into())
}

pub fn describe(items: &[MailItem]) -> String {
    if items.is_empty() {
        return "Nenhum e-mail encontrado.".into();
    }
    items
        .iter()
        .map(|m| format!("[{}] {}{} — {} ({})\n  {}", m.id, if m.unread { "• " } else { "" }, m.from, m.subject, m.date, m.snippet))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Tab commands (only when the E-mail tab is opened or used) ─────────────────

#[tauri::command]
pub async fn gmail_list(query: String) -> Result<Vec<MailItem>, String> {
    list(&query, 20).await
}

#[tauri::command]
pub async fn gmail_read(id: String) -> Result<MailFull, String> {
    read(&id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_round_trips() {
        let raw = build_raw("ana@x.com", "Olá", "corpo", None);
        let back = String::from_utf8(b64url_decode(&raw)).unwrap();
        assert!(back.contains("To: ana@x.com"));
        assert!(back.contains("=?UTF-8?B?"));
        assert!(back.ends_with("corpo"));
    }

    #[test]
    fn html_mail_reads_as_text() {
        let t = html_to_text("<style>p{}</style><p>Oi &amp; tchau</p><br>fim");
        assert!(t.starts_with("Oi & tchau\n") && t.ends_with("\nfim") && !t.contains("p{}"));
    }

    #[test]
    fn reply_goes_to_the_sender_address() {
        assert_eq!(address("Ana Souza <ana@x.com>"), "ana@x.com");
        assert_eq!(address("bob@y.com"), "bob@y.com");
    }
}
