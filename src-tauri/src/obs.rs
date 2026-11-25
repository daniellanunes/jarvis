//! OBS Studio by voice, through OBS's own WebSocket server (OBS 28 or newer:
//! Ferramentas → Configurações do servidor WebSocket → Ativar). Recording,
//! the live stream, scenes, the microphone, the replay buffer.
//!
//! "Começa a gravar", "encerra a live", "troca para a cena Câmera",
//! "muta o microfone do OBS". The password (when OBS asks for one) lives in
//! the Windows vault (`obs-password`), set in Ajustes.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

use crate::file_search::fold;

pub const PASSWORD_KEY: &str = "obs-password";
const URL: &str = "ws://127.0.0.1:4455";

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// obs-websocket 5: base64(sha256(base64(sha256(password + salt)) + challenge)).
fn auth(password: &str, salt: &str, challenge: &str) -> String {
    let secret = b64(&Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64(&Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

/// Whether OBS is running on this PC.
pub fn running() -> bool {
    let p = crate::routines::running_programs();
    p.contains("obs64") || p.contains("obs32") || p.contains("obs")
}

/// Sends the requests in order, on one connection; their `responseData`.
async fn requests(list: &[(&str, Value)]) -> Result<Vec<Value>, String> {
    let not_reached = "Não consegui falar com o OBS. Ele está aberto? No OBS: Ferramentas → Configurações do servidor WebSocket → Ativar servidor.";
    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(3), tokio_tungstenite::connect_async(URL))
        .await
        .map_err(|_| not_reached.to_string())?
        .map_err(|_| not_reached.to_string())?;

    async fn next(ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> Result<Value, String> {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
                Err(_) => return Err("O OBS não respondeu.".into()),
                Ok(None) => return Err("O OBS fechou a conexão.".into()),
                Ok(Some(Err(e))) => return Err(format!("OBS: {e}")),
                Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(&t).map_err(|e| e.to_string()),
                Ok(Some(Ok(Message::Close(frame)))) => {
                    let code = frame.map(|f| u16::from(f.code)).unwrap_or(0);
                    return Err(if code == 4009 {
                        "O OBS recusou a senha. Confira a senha em Ajustes → IA & Voz → OBS.".into()
                    } else {
                        format!("O OBS fechou a conexão ({code}).")
                    });
                }
                Ok(Some(Ok(_))) => continue,
            }
        }
    }

    let hello = next(&mut ws).await?;
    let mut identify = json!({ "rpcVersion": 1, "eventSubscriptions": 0 });
    if let Some(a) = hello["d"].get("authentication") {
        let password = crate::secrets::get(PASSWORD_KEY).ok_or("O OBS pede senha. Cole a senha do servidor WebSocket em Ajustes → IA & Voz → OBS.")?;
        identify["authentication"] = json!(auth(&password, a["salt"].as_str().unwrap_or(""), a["challenge"].as_str().unwrap_or("")));
    }
    ws.send(Message::Text(json!({ "op": 1, "d": identify }).to_string())).await.map_err(|e| e.to_string())?;
    let identified = next(&mut ws).await?;
    if identified["op"].as_i64() != Some(2) {
        return Err("O OBS não aceitou a conexão.".into());
    }

    let mut out = Vec::new();
    for (i, (kind, data)) in list.iter().enumerate() {
        let id = format!("jv{i}");
        ws.send(Message::Text(json!({ "op": 6, "d": { "requestType": kind, "requestId": id, "requestData": data } }).to_string()))
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let v = next(&mut ws).await?;
            if v["op"].as_i64() == Some(7) && v["d"]["requestId"].as_str() == Some(id.as_str()) {
                let status = &v["d"]["requestStatus"];
                if !status["result"].as_bool().unwrap_or(false) {
                    let code = status["code"].as_i64().unwrap_or(0);
                    let comment = status["comment"].as_str().unwrap_or("");
                    return Err(match code {
                        // Already in the state asked for: say it plainly.
                        500 => "O OBS já está assim.".into(),
                        501 => "Isso não está ligado no OBS agora.".into(),
                        _ => format!("O OBS não aceitou ({kind}): {comment}"),
                    });
                }
                out.push(v["d"]["responseData"].clone());
                break;
            }
        }
    }
    let _ = ws.close(None).await;
    Ok(out)
}

/// The scene whose name best matches what was said.
fn pick_scene(scenes: &[String], said: &str) -> Option<String> {
    let w = fold(said.trim());
    if w.is_empty() {
        return None;
    }
    scenes
        .iter()
        .find(|s| fold(s) == w)
        .or_else(|| scenes.iter().find(|s| fold(s).starts_with(&w)))
        .or_else(|| scenes.iter().find(|s| fold(s).contains(&w) || w.contains(&fold(s))))
        .cloned()
}

/// Does one thing in OBS and says how it went.
pub async fn act(action: &str, value: &str) -> Result<String, String> {
    match action {
        "record_start" => requests(&[("StartRecord", json!({}))]).await.map(|_| "Gravando no OBS.".into()),
        "record_stop" => requests(&[("StopRecord", json!({}))]).await.map(|r| {
            let path = r.first().and_then(|v| v["outputPath"].as_str()).unwrap_or("");
            if path.is_empty() { "Gravação encerrada.".into() } else { format!("Gravação salva em {path}.") }
        }),
        "record_pause" => requests(&[("PauseRecord", json!({}))]).await.map(|_| "Gravação pausada.".into()),
        "record_resume" => requests(&[("ResumeRecord", json!({}))]).await.map(|_| "Gravação retomada.".into()),
        "stream_start" => requests(&[("StartStream", json!({}))]).await.map(|_| "A live começou.".into()),
        "stream_stop" => requests(&[("StopStream", json!({}))]).await.map(|_| "Live encerrada.".into()),
        "replay_save" => requests(&[("SaveReplayBuffer", json!({}))]).await.map(|_| "Replay salvo.".into()),
        "virtualcam_start" => requests(&[("StartVirtualCam", json!({}))]).await.map(|_| "Câmera virtual ligada.".into()),
        "virtualcam_stop" => requests(&[("StopVirtualCam", json!({}))]).await.map(|_| "Câmera virtual desligada.".into()),
        "scene" => {
            let list = requests(&[("GetSceneList", json!({}))]).await?;
            let scenes: Vec<String> = list[0]["scenes"].as_array().map(|a| a.iter().filter_map(|s| s["sceneName"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            let Some(scene) = pick_scene(&scenes, value) else {
                return Err(format!("Não achei a cena \"{value}\". Cenas: {}.", scenes.join(", ")));
            };
            requests(&[("SetCurrentProgramScene", json!({ "sceneName": scene }))]).await.map(|_| format!("Cena {scene}."))
        }
        "scenes" => {
            let list = requests(&[("GetSceneList", json!({}))]).await?;
            let scenes: Vec<String> = list[0]["scenes"].as_array().map(|a| a.iter().filter_map(|s| s["sceneName"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            Ok(format!("Cenas do OBS: {}. Agora: {}.", scenes.join(", "), list[0]["currentProgramSceneName"].as_str().unwrap_or("?")))
        }
        "mute" | "unmute" => {
            let list = requests(&[("GetInputList", json!({}))]).await?;
            let inputs: Vec<(String, String)> = list[0]["inputs"]
                .as_array()
                .map(|a| a.iter().map(|i| (i["inputName"].as_str().unwrap_or("").to_string(), i["inputKind"].as_str().unwrap_or("").to_string())).collect())
                .unwrap_or_default();
            // Named, else the microphone (an audio input capture).
            let name = if value.trim().is_empty() {
                inputs
                    .iter()
                    .find(|(_, k)| k.contains("input_capture"))
                    .or_else(|| inputs.iter().find(|(n, _)| fold(n).contains("mic")))
                    .map(|(n, _)| n.clone())
            } else {
                let names: Vec<String> = inputs.iter().map(|(n, _)| n.clone()).collect();
                pick_scene(&names, value)
            };
            let Some(name) = name else { return Err("Não achei o microfone no OBS.".into()) };
            let mute = action == "mute";
            requests(&[("SetInputMute", json!({ "inputName": name, "inputMuted": mute }))])
                .await
                .map(|_| format!("{name} {} no OBS.", if mute { "mudo" } else { "ligado" }))
        }
        "status" => {
            let r = requests(&[("GetRecordStatus", json!({})), ("GetStreamStatus", json!({})), ("GetCurrentProgramScene", json!({}))]).await?;
            let rec = if r[0]["outputActive"].as_bool().unwrap_or(false) {
                format!("gravando há {}", r[0]["outputTimecode"].as_str().unwrap_or("").split('.').next().unwrap_or(""))
            } else {
                "sem gravar".into()
            };
            let live = if r[1]["outputActive"].as_bool().unwrap_or(false) {
                format!("ao vivo há {}", r[1]["outputTimecode"].as_str().unwrap_or("").split('.').next().unwrap_or(""))
            } else {
                "sem live".into()
            };
            Ok(format!("OBS: {rec}, {live}, cena {}.", r[2]["currentProgramSceneName"].as_str().or(r[2]["sceneName"].as_str()).unwrap_or("?")))
        }
        _ => Err(format!("Não sei fazer \"{action}\" no OBS.")),
    }
}

static BUSY: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);

/// Recording or live right now (asked at most every 15 s). False when OBS is
/// closed or does not answer.
pub async fn on_air() -> bool {
    if let Some((at, v)) = *BUSY.lock().unwrap_or_else(|e| e.into_inner()) {
        if at.elapsed() < Duration::from_secs(15) {
            return v;
        }
    }
    let v = if !running() {
        false
    } else {
        match requests(&[("GetRecordStatus", json!({})), ("GetStreamStatus", json!({}))]).await {
            Ok(r) => r.iter().any(|x| x["outputActive"].as_bool().unwrap_or(false)),
            Err(_) => false,
        }
    };
    *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), v));
    v
}

/// What was said → an OBS action. None = not about OBS.
pub fn parse(said: &str) -> Option<(&'static str, String)> {
    let t: String = fold(said).chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' }).collect();
    let mut t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    for pre in ["jarvis ", "por favor ", "pode "] {
        if let Some(rest) = t.strip_prefix(pre) {
            t = rest.to_string();
        }
    }
    let has = |w: &[&str]| w.iter().any(|x| t.contains(x));
    let obs = t.split(' ').any(|w| w == "obs");
    let rec = has(&["gravar", "gravacao", "gravando", "grava "]) || t.ends_with("grava");
    let live = has(&["live", "transmissao", "ao vivo", "stream"]);
    let start = has(&["comeca", "comecar", "comece", "inicia", "iniciar", "inicie", "liga a", "abre a", "entra", "bora"]);
    let stop = has(&["para ", "parar", "pare ", "termina", "terminar", "encerra", "encerrar", "finaliza", "finalizar", "desliga a", "sai do", "sair do"]) || t.starts_with("para");

    if has(&["replay"]) && has(&["salva", "salvar", "grava", "clipa"]) {
        return Some(("replay_save", String::new()));
    }
    if obs && has(&["microfone", " mic"]) {
        let off = has(&["muta", "mutar", "silencia", "desliga", "mudo", "corta"]);
        return Some((if off { "mute" } else { "unmute" }, String::new()));
    }
    if obs && has(&["camera virtual"]) {
        return Some((if stop || has(&["desliga"]) { "virtualcam_stop" } else { "virtualcam_start" }, String::new()));
    }
    if has(&["pausa a gravacao", "pausar a gravacao", "pausa a gravação"]) {
        return Some(("record_pause", String::new()));
    }
    if has(&["retoma a gravacao", "continua a gravacao", "continua gravando", "volta a gravar"]) {
        return Some(("record_resume", String::new()));
    }
    if rec && stop {
        return Some(("record_stop", String::new()));
    }
    if rec && (start || t.starts_with("grava")) && !has(&["audio", "mensagem", "whatsapp", "nota"]) {
        return Some(("record_start", String::new()));
    }
    if live && stop {
        return Some(("stream_stop", String::new()));
    }
    if live && start {
        return Some(("stream_start", String::new()));
    }
    // "Troca para a cena Câmera", "muda a cena pra jogo", "cena tela cheia".
    if let Some(i) = t.find("cena ") {
        let mut name = t[i + 5..].trim();
        for pre in ["pra ", "para ", "de ", "a ", "o ", "da ", "do "] {
            name = name.strip_prefix(pre).unwrap_or(name).trim();
        }
        let name = name.trim_end_matches(" no obs").trim();
        if !name.is_empty() && (obs || i == 0 || has(&["troca", "muda", "mudar", "trocar", "vai para", "vai pra", "coloca", "poe"])) {
            // The scene as it was said (accents and capitals).
            let words = name.split(' ').count();
            let original: Vec<&str> = said.split(|c: char| c.is_whitespace()).filter(|w| !w.is_empty()).collect();
            let pretty = original
                .windows(words.min(original.len()).max(1))
                .rev()
                .find(|w| fold(&w.join(" ")).replace(|c: char| !c.is_alphanumeric() && c != ' ', "") == name)
                .map(|w| w.join(" "))
                .unwrap_or_else(|| name.to_string());
            return Some(("scene", pretty));
        }
    }
    if obs && has(&["cenas", "quais cenas"]) {
        return Some(("scenes", String::new()));
    }
    if obs && has(&["status", "como esta", "esta gravando", "ta gravando", "ao vivo"]) {
        return Some(("status", String::new()));
    }
    None
}

/// An OBS command said to Jarvis: done at once. Recording and live words
/// without "OBS" only count while OBS is open.
pub async fn quick(said: &str) -> Option<String> {
    let (action, value) = parse(said)?;
    let named = fold(said).split(|c: char| !c.is_alphanumeric()).any(|w| w == "obs");
    let open = tokio::task::spawn_blocking(running).await.unwrap_or(false);
    if !open {
        return named.then(|| "O OBS não está aberto. Abra ele primeiro (ou diga \"abre o OBS\").".into());
    }
    *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Some(match act(action, &value).await {
        Ok(msg) => msg,
        Err(e) => e,
    })
}

/// A routine's `obs:` step: "gravar", "parar", "live", "parar live",
/// "cena Câmera", "mutar", "desmutar", "replay". Waits for OBS when the same
/// routine just opened it.
pub async fn step(words: &str) -> Result<String, String> {
    let w = fold(words.trim());
    let (action, value): (&str, String) = match w.as_str() {
        "gravar" | "gravacao" | "record" | "comecar a gravar" | "iniciar gravacao" => ("record_start", String::new()),
        "parar" | "parar gravacao" | "parar de gravar" | "stop" | "encerrar gravacao" => ("record_stop", String::new()),
        "live" | "stream" | "transmitir" | "comecar live" | "iniciar live" => ("stream_start", String::new()),
        "parar live" | "encerrar live" | "fim da live" | "parar transmissao" => ("stream_stop", String::new()),
        "mutar" | "mudo" | "mutar microfone" => ("mute", String::new()),
        "desmutar" | "microfone" | "ligar microfone" => ("unmute", String::new()),
        "replay" | "salvar replay" => ("replay_save", String::new()),
        "camera virtual" => ("virtualcam_start", String::new()),
        _ if w.starts_with("cena ") => ("scene", words.trim().chars().skip(5).collect::<String>().trim().to_string()),
        _ => parse(&format!("{words} no obs")).ok_or_else(|| format!("Passo do OBS desconhecido: {words}"))?,
    };
    for _ in 0..25 {
        if running() {
            match act(action, &value).await {
                Err(e) if e.starts_with("Não consegui falar") => {}
                other => return other,
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("O OBS não respondeu (ele está aberto, com o servidor WebSocket ativo?).".into())
}

/// For Ajustes: whether OBS answers (and how).
#[tauri::command]
pub async fn obs_test() -> Result<String, String> {
    if !tokio::task::spawn_blocking(running).await.unwrap_or(false) {
        return Err("O OBS não está aberto.".into());
    }
    act("status", "").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_answer_matches_obs_websocket() {
        // The example in the obs-websocket 5 protocol docs.
        let a = auth("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=");
        assert_eq!(a, "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=");
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"M"), "TQ==");
    }

    #[test]
    fn spoken_obs_commands() {
        assert_eq!(parse("Jarvis, começa a gravar"), Some(("record_start", String::new())));
        assert_eq!(parse("inicia a gravação no OBS"), Some(("record_start", String::new())));
        assert_eq!(parse("para de gravar"), Some(("record_stop", String::new())));
        assert_eq!(parse("encerra a gravação"), Some(("record_stop", String::new())));
        assert_eq!(parse("começa a live"), Some(("stream_start", String::new())));
        assert_eq!(parse("encerra a transmissão"), Some(("stream_stop", String::new())));
        assert_eq!(parse("troca para a cena Câmera"), Some(("scene", "Câmera".into())));
        assert_eq!(parse("muda a cena pra Tela Cheia no OBS"), Some(("scene", "Tela Cheia".into())));
        assert_eq!(parse("muta o microfone do OBS"), Some(("mute", String::new())));
        assert_eq!(parse("liga o microfone do OBS"), Some(("unmute", String::new())));
        assert_eq!(parse("salva o replay"), Some(("replay_save", String::new())));
        assert_eq!(parse("pausa a gravação"), Some(("record_pause", String::new())));
        // Not OBS.
        assert_eq!(parse("grava um áudio no WhatsApp"), None);
        assert_eq!(parse("desliga o microfone"), None);
        assert_eq!(parse("abre o OBS"), None);
        assert_eq!(parse("toca uma música"), None);
    }

    #[test]
    fn scenes_are_found_by_what_was_said() {
        let s: Vec<String> = ["Câmera", "Tela Cheia", "Jogo + Cam", "Intervalo"].iter().map(|x| x.to_string()).collect();
        assert_eq!(pick_scene(&s, "camera").as_deref(), Some("Câmera"));
        assert_eq!(pick_scene(&s, "tela").as_deref(), Some("Tela Cheia"));
        assert_eq!(pick_scene(&s, "jogo").as_deref(), Some("Jogo + Cam"));
        assert_eq!(pick_scene(&s, "nada"), None);
    }
}
