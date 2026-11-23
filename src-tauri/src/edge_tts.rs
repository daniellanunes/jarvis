//! The Jarvis voice: Microsoft's neural voices (the ones Edge's "Read aloud"
//! uses), spoken natively — no Python, no edge-tts package. Before this the
//! voice came from `python -m edge_tts`; on a PC without Python the reply fell
//! back to the old Windows voice, the "robotic" one.
//!
//! The service wants, besides the voice request itself, a "Sec-MS-GEC" token:
//! a hash of the current 5-minute slot. It is a faithful port of edge-tts's
//! drm.py (v2: an SHA-256 with a hidden prefix plus a "CS64" checksum); the
//! tests compare it with values produced by the Python package.

use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WSS_URL: &str =
    "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
const CHROMIUM_MAJOR_VERSION: &str = "143";

/// The voices offered in Ajustes: (id, label). Portuguese first, then the
/// multilingual ones, which also speak Portuguese.
pub const VOICES: &[(&str, &str)] = &[
    ("pt-BR-AntonioNeural", "Antonio — masculina, Brasil"),
    ("pt-BR-FranciscaNeural", "Francisca — feminina, Brasil"),
    ("pt-BR-ThalitaMultilingualNeural", "Thalita — feminina, Brasil"),
    ("pt-PT-DuarteNeural", "Duarte — masculina, Portugal"),
    ("pt-PT-RaquelNeural", "Raquel — feminina, Portugal"),
    ("en-US-AndrewMultilingualNeural", "Andrew — masculina, multilíngue"),
    ("en-US-BrianMultilingualNeural", "Brian — masculina, multilíngue"),
    ("en-US-AvaMultilingualNeural", "Ava — feminina, multilíngue"),
    ("en-US-EmmaMultilingualNeural", "Emma — feminina, multilíngue"),
    ("fr-FR-RemyMultilingualNeural", "Remy — masculina, multilíngue"),
    ("fr-FR-VivienneMultilingualNeural", "Vivienne — feminina, multilíngue"),
];

pub const DEFAULT_VOICE: &str = "pt-BR-AntonioNeural";

// ── Sec-MS-GEC (port of edge_tts/drm.py, generate_sec_ms_gec_v2) ─────────────

const M: u64 = 0xFFFF_FFFF;
const WIN_EPOCH: f64 = 11_644_473_600.0;

const SHA256_INITIAL: [u64; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
const SHA256_STATE: [u64; 8] = [0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4, 0x03EFFBD2];
const SHA256_SCHEDULE: [u64; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0, 0xF9E70AB6, 0xF0C31573, 0x7058CE4E, 0x7188E606, 0x00E5AB33,
    0xD258B060, 0x72766564,
];
const K: [u64; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5, 0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3,
    0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174, 0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA,
    0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967, 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13,
    0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85, 0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070,
    0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3, 0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208,
    0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2,
];
const SHA256_ADJUSTMENT: u64 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u64 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u64 = 0x000B92C0;
const CS64_COEFFICIENTS: [(u64, u64); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u64 = 0x82893933;
const CS64_ADJUSTMENT0: u64 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u64 = 0x0B1A0000;

fn rol16(v: u64) -> u64 {
    let v = v & M;
    ((v << 16) | (v >> 16)) & M
}

fn sigma0(v: u64) -> u64 {
    ((v >> 7) ^ (v << 25) ^ (v >> 18) ^ (v << 14) ^ (v >> 3)) & M
}

fn sigma1(v: u64) -> u64 {
    ((v >> 17) ^ (v << 15) ^ (v >> 19) ^ (v << 13) ^ (v >> 10)) & M
}

/// SHA-256 rounds from `first_round` (14 resumes inside the hidden first block,
/// with one round constant changed).
fn sha256_rounds(state: [u64; 8], words: &[u64], first_round: usize) -> [u64; 8] {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for i in first_round..64 {
        let k = if first_round == 14 && i == 14 { SHA256_ADJUSTMENT } else { K[i] };
        let rot_e = e.wrapping_mul(0x1_0000_0001);
        let rot_a = a.wrapping_mul(0x1_0000_0001);
        let t1 = h
            .wrapping_add((rot_e >> 6) ^ (rot_e >> 11) ^ (rot_e >> 25))
            .wrapping_add(g ^ (e & (f ^ g)))
            .wrapping_add(k)
            .wrapping_add(words[i]);
        let t2 = ((rot_a >> 2) ^ (rot_a >> 13) ^ (rot_a >> 22)).wrapping_add((a & b) | (c & (a | b)));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1) & M;
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2) & M;
    }
    [a, b, c, d, e, f, g, h]
}

fn expand_schedule(words: &mut Vec<u64>, first_word: usize) {
    for i in first_word..64 {
        let (x, y) = (words[i - 2], words[i - 15]);
        let s1 = (x >> 17) ^ (x << 15) ^ (x >> 19) ^ (x << 13) ^ (x >> 10);
        let s0 = (y >> 7) ^ (y << 25) ^ (y >> 18) ^ (y << 14) ^ (y >> 3);
        words.push(s1.wrapping_add(words[i - 7]).wrapping_add(s0).wrapping_add(words[i - 16]) & M);
    }
}

fn sha256_init(data: &[u8]) -> [u64; 8] {
    let be = |b: &[u8]| b.iter().fold(0u64, |acc, x| (acc << 8) | *x as u64);
    let mut words: Vec<u64> = vec![0; 14];
    words.push(be(&data[..3]));
    words.push(be(&data[3..7]));
    for i in 16..30 {
        let mut value = SHA256_SCHEDULE[i - 16];
        if i - 2 >= 14 {
            let s = sigma1(words[i - 2]);
            value = value.wrapping_add(if i == 16 { s ^ SHA256_SIGMA1_ADJUSTMENT } else { s });
        }
        if i - 7 >= 14 {
            value = value.wrapping_add(words[i - 7]);
        }
        if i - 15 >= 14 {
            let s = sigma0(words[i - 15]);
            value = value.wrapping_add(if i == 29 { s ^ SHA256_SIGMA0_ADJUSTMENT } else { s });
        }
        words.push(value & M);
    }
    words.push(
        sigma1(words[28])
            .wrapping_add(words[23])
            .wrapping_add(sigma0(words[15]))
            .wrapping_add(words[14])
            .wrapping_add(SHA256_ADJUSTMENT)
            .wrapping_sub(K[14])
            & M,
    );
    expand_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    let mut out = [0u64; 8];
    for i in 0..8 {
        out[i] = SHA256_INITIAL[i].wrapping_add(working[i]) & M;
    }
    out
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let length = 57 + data.len();
    let mut padded = data[7..].to_vec();
    padded.push(0x80);
    let zeros = (55i64 - length as i64).rem_euclid(64) as usize;
    padded.extend(std::iter::repeat(0u8).take(zeros));
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());
    let mut state = sha256_init(&data[..7]);
    for block in padded.chunks(64) {
        let mut words: Vec<u64> = block.chunks(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as u64).collect();
        expand_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]) & M;
        }
    }
    let mut out = [0u8; 32];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&(*w as u32).to_be_bytes());
    }
    out
}

fn words_le4_even(data: &[u8]) -> Vec<u64> {
    let mut padded = data.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    let mut words: Vec<u64> = padded.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64).collect();
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u64, u64) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) as u64 | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) as u64 | 1;
    let factors = [key_a.wrapping_mul(CS64_FACTOR) & M, key_b.wrapping_mul(CS64_FACTOR) & M];
    let mut terms: Vec<u64> = CS64_COEFFICIENTS.iter().enumerate().map(|(i, (c, _))| c.wrapping_mul(factors[i % 2]) & M).collect();
    let mut offsets: Vec<u64> = CS64_COEFFICIENTS.iter().map(|(_, o)| *o).collect();
    let mut with_zero = vec![0u8];
    with_zero.extend_from_slice(data);
    for (i, word) in words_le4_even(&with_zero).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if i % 2 == 0 { key_a } else { key_b });
        let mut offset = rol16(word).wrapping_mul(if i % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if i == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term & M);
        offsets.push(offset & M);
    }
    let (mut swap_acc, mut swap_last, mut state, mut acc) = (0u64, 0u64, 0u64, 0u64);
    for i in (0..terms.len()).step_by(2) {
        let (x, y) = (terms[i], terms[i + 1]);
        let t = rol16(offsets[i].wrapping_add(x) & M);
        let q = t.wrapping_mul(0x76F50F34) & M;
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q) & M);
        let t = rol16(offsets[i + 1].wrapping_add(y) & M);
        let q = t.wrapping_mul(0x1881CE3C) & M;
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q) & M;
        swap_acc = swap_acc.wrapping_add(swap_last) & M;

        state = state.wrapping_mul(key_a).wrapping_add(x) & M;
        state = rol16(state).wrapping_mul(0xCF98B111) & M;
        state = rol16(state).wrapping_mul(0x87085B9F) & M;
        state = rol16(state).wrapping_mul(0x12CEB96D) & M;
        state = rol16(state).wrapping_mul(0x257E1D83) & M;
        acc = acc.wrapping_add(state);
        let mut t = state.wrapping_mul(key_b).wrapping_add(y) & M;
        t = rol16(t).wrapping_mul(0xA27416F5) & M;
        t = rol16(t).wrapping_mul(0xD38396FF) & M;
        state = rol16(rol16(t).wrapping_mul(0x7C932B89) & M).wrapping_mul(0xBFA49F69) & M;
        acc = acc.wrapping_add(state) & M;
    }
    (swap_last ^ state, swap_acc ^ acc)
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// The token for `unix` seconds (the current 5-minute slot).
pub fn sec_ms_gec(unix: f64) -> String {
    let mut ticks = unix + WIN_EPOCH;
    ticks -= ticks % 300.0;
    ticks *= 1e9 / 100.0;
    let data = format!("{ticks:.0}{TRUSTED_CLIENT_TOKEN}");
    let digest = sha256_digest(data.as_bytes());
    let (a, b) = cs64_digest(data.as_bytes(), &digest);
    let mut cs = (a as u32).to_le_bytes().to_vec();
    cs.extend_from_slice(&(b as u32).to_le_bytes());
    format!("{}-{}", hex_upper(&cs), hex_upper(&digest))
}

// ── Speaking ─────────────────────────────────────────────────────────────────

/// Seconds to add to the PC's clock (set from the server's Date when refused).
static CLOCK_SKEW: AtomicI64 = AtomicI64::new(0);

fn now_unix() -> f64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    t + CLOCK_SKEW.load(Ordering::Relaxed) as f64
}

fn random_hex(n: usize, upper: bool) -> String {
    let mut b = vec![0u8; n];
    let _ = getrandom::getrandom(&mut b);
    let s = hex_upper(&b);
    if upper {
        s
    } else {
        s.to_lowercase()
    }
}

fn js_date() -> String {
    chrono::Utc::now().format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c if (c as u32) < 32 && c != '\n' => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Splits long text at sentence or word ends (the service takes ~4 KB per request).
fn chunks(text: &str, max_bytes: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while rest.len() > max_bytes {
        let mut cut = max_bytes;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let head = &rest[..cut];
        let at = head.rfind(['.', '!', '?', '\n']).map(|i| i + 1).or_else(|| head.rfind(' ')).filter(|i| *i > max_bytes / 3).unwrap_or(cut);
        out.push(rest[..at].trim().to_string());
        rest = rest[at..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// One request: the MP3 of `text`.
async fn synth_once(text: &str, voice: &str, rate_pct: i32) -> Result<Vec<u8>, String> {
    let url = format!(
        "{WSS_URL}&ConnectionId={}&Sec-MS-GEC={}&Sec-MS-GEC-Version=2-{CHROMIUM_FULL_VERSION}",
        random_hex(16, false),
        sec_ms_gec(now_unix())
    );
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    let h = req.headers_mut();
    let ua = format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{v}.0.0.0 Safari/537.36 Edg/{v}.0.0.0",
        v = CHROMIUM_MAJOR_VERSION
    );
    for (k, v) in [
        ("Pragma", "no-cache".to_string()),
        ("Cache-Control", "no-cache".to_string()),
        ("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".to_string()),
        ("User-Agent", ua),
        ("Accept-Encoding", "gzip, deflate, br, zstd".to_string()),
        ("Accept-Language", "en-US,en;q=0.9".to_string()),
        ("Cookie", format!("muid={};", random_hex(16, true))),
    ] {
        h.insert(k, HeaderValue::from_str(&v).map_err(|e| e.to_string())?);
    }

    let connect = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| "Voz: o serviço não respondeu.".to_string())?;
    let (mut ws, _) = match connect {
        Ok(ok) => ok,
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            // A wrong PC clock gives 403: learn the offset from the server.
            if let Some(date) = resp.headers().get("Date").and_then(|d| d.to_str().ok()) {
                if let Ok(server) = chrono::DateTime::parse_from_rfc2822(date) {
                    let skew = server.timestamp() as f64 - now_unix();
                    CLOCK_SKEW.fetch_add(skew as i64, Ordering::Relaxed);
                }
            }
            return Err(format!("Voz: o serviço recusou ({})", resp.status()));
        }
        Err(e) => return Err(format!("Voz: {e}")),
    };

    ws.send(Message::Text(format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{}\r\n",
        js_date(),
        r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"false"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#
    )))
    .await
    .map_err(|e| e.to_string())?;
    let rate = format!("{}{}%", if rate_pct >= 0 { "+" } else { "" }, rate_pct);
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{voice}'><prosody pitch='+0Hz' rate='{rate}' volume='+0%'>{}</prosody></voice></speak>",
        escape(text)
    );
    ws.send(Message::Text(format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n{ssml}",
        random_hex(16, false),
        js_date()
    )))
    .await
    .map_err(|e| e.to_string())?;

    let mut audio = Vec::new();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .map_err(|_| "Voz: o serviço parou de responder.".to_string())?
            .ok_or("Voz: conexão fechada antes do fim.")?
            .map_err(|e| e.to_string())?;
        match msg {
            Message::Text(t) if t.contains("Path:turn.end") => break,
            Message::Binary(data) if data.len() >= 2 => {
                let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
                if header_len + 2 <= data.len() {
                    let headers = String::from_utf8_lossy(&data[..header_len]);
                    if headers.contains("Path:audio") {
                        audio.extend_from_slice(&data[header_len + 2..]);
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    if audio.is_empty() {
        return Err("Voz: nenhum áudio recebido (voz inválida?).".into());
    }
    Ok(audio)
}

/// The MP3 of `text` in `voice`, `rate_pct` faster (negative = slower).
pub async fn synthesize(text: &str, voice: &str, rate_pct: i32) -> Result<Vec<u8>, String> {
    let voice = if VOICES.iter().any(|(id, _)| *id == voice) { voice } else { DEFAULT_VOICE };
    let mut out = Vec::new();
    for part in chunks(text, 3000) {
        // One retry: the first refusal may have just corrected the clock.
        let audio = match synth_once(&part, voice, rate_pct).await {
            Ok(a) => a,
            Err(_) => synth_once(&part, voice, rate_pct).await?,
        };
        out.extend(audio);
    }
    Ok(out)
}

#[derive(serde::Serialize)]
pub struct VoiceOption {
    id: &'static str,
    label: &'static str,
}

/// The voices for Ajustes.
#[tauri::command]
pub fn tts_voices() -> Vec<VoiceOption> {
    VOICES.iter().map(|(id, label)| VoiceOption { id, label }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from the Python package (edge_tts 7.3.1, generate_sec_ms_gec_v2).
    #[test]
    fn the_token_matches_edge_tts() {
        let cases = [
            (0.0, "52BC7A17B50DFEC9-28C4394775C8751CD2F6859C030C06DB1489ACAB3B3A7651831FC26895177ED6"),
            (1700000000.0, "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"),
            (1795400000.5, "1A952AB8BA4AECC5-06FD9CFB946E118BB1AB87A012E0465BEF195ADCCE975716963AE80477F78F45"),
            (1795412345.0, "2091302142567032-66F0126C650EE98135E27E9D23A11CAAEDA19DD334AF1D32830EDE7BE9319570"),
            (2000000000.0, "2D62557F1142BC5D-FD9C30C68FF194A5EE6B45FEFC5E2BA1D1239F88A90569AB55CFFFAA7799F40E"),
        ];
        for (ts, want) in cases {
            assert_eq!(sec_ms_gec(ts), want, "ts={ts}");
        }
    }

    #[test]
    fn long_text_is_split_at_sentences() {
        let text = "Frase um. ".repeat(500);
        let parts = chunks(&text, 3000);
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| p.len() <= 3000 && p.ends_with('.')));
        assert_eq!(chunks("Oi.", 3000), vec!["Oi.".to_string()]);
    }

    #[test]
    fn text_is_escaped_for_ssml() {
        assert_eq!(escape("a & <b> 'c'"), "a &amp; &lt;b&gt; &apos;c&apos;");
    }

    /// Real request (`cargo test speaks_for_real -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn speaks_for_real() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mp3 = rt.block_on(synthesize("Olá! Eu sou o Jarvis.", "pt-BR-FranciscaNeural", 8)).unwrap();
        println!("{} bytes de MP3", mp3.len());
        assert!(mp3.len() > 4000);
        std::fs::write(std::env::temp_dir().join("jarvis-tts-test.mp3"), mp3).unwrap();
    }
}
