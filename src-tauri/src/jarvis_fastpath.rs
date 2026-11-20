//! Instant commands: simple, unambiguous requests ("volume 30", "pausa",
//! "próxima", "toca Dawn FM", "abre o YouTube") run directly, in milliseconds,
//! without asking any AI. Anything compound or unclear returns None and goes
//! to the brain as before.

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Volume(u32),
    VolumeDelta(i32),
    Mute,
    Media(&'static str),
    SpotifyPlay(String),
    OpenApp(String),
    OpenSite(String),
    Search(String),
    /// Read the agenda: (days from today, how many days).
    Agenda(i64, u32),
    /// "Bom dia": weather, agenda and tasks.
    Briefing,
    /// Mute (true) or unmute the microphone.
    Mic(bool),
    /// Switch the audio output ("fone", "caixa"…; "" = the next one).
    Output(String),
}

/// "o que eu tenho hoje", "minha agenda de amanhã", "compromissos da semana"…
fn agenda_intent(key: &str) -> Option<Intent> {
    let about_agenda = ["agenda", "compromisso", "compromissos", "reuniao", "reunioes", "eventos", "evento"]
        .iter()
        .any(|w| key.split_whitespace().any(|k| k == *w))
        || key.starts_with("o que eu tenho")
        || key.starts_with("o que tenho")
        || key.starts_with("tenho algo")
        || key.starts_with("tenho alguma coisa");
    // Creating or changing goes to the brain ("marca reunião amanhã às 15h").
    let changes = ["marca", "marque", "agenda uma", "agende", "cria", "crie", "adiciona", "coloca", "coloque", "cancela", "cancele", "apaga", "apague", "muda", "mude", "move", "remarca", "desmarca"];
    if !about_agenda || changes.iter().any(|c| key.starts_with(c) || key.contains(&format!(" {c} "))) {
        return None;
    }
    if key.contains("semana") {
        return Some(Intent::Agenda(0, 7));
    }
    if key.contains("depois de amanha") {
        return Some(Intent::Agenda(2, 1));
    }
    if key.contains("amanha") {
        return Some(Intent::Agenda(1, 1));
    }
    if key.contains("hoje") || key == "minha agenda" || key == "agenda" || key.starts_with("o que eu tenho") || key.starts_with("o que tenho") {
        return Some(Intent::Agenda(0, 1));
    }
    None
}

fn strip_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'ê' | 'è' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        c => c,
    }
}

/// (original lowercase text, same text without accents) — same char count, so
/// a position found in the second slices the first.
fn prepare(text: &str) -> (Vec<char>, String) {
    let lower: String = text
        .to_lowercase()
        .chars()
        .map(|c| if matches!(c, ',' | '!' | '?' | ';') { ' ' } else { c })
        .collect();
    let trimmed = lower.trim().trim_end_matches('.').trim().to_string();
    let chars: Vec<char> = trimmed.chars().collect();
    let key: String = chars.iter().map(|c| strip_accent(*c)).collect();
    (chars, key)
}

const POLITE: &[&str] = &[
    "ei jarvis ", "ok jarvis ", "oi jarvis ", "hey jarvis ", "e ai jarvis ", "jarvis ", "por favor ", "pfv ",
    "voce pode ", "voce consegue ", "consegue ", "pode ", "da pra ", "quero que voce ", "eu quero ", "quero ",
];

/// Drops greetings and politeness from the front; returns the char offset kept.
fn skip_polite(key: &str) -> usize {
    let mut offset = 0;
    loop {
        let rest = &key[byte_at(key, offset)..];
        match POLITE.iter().find(|p| rest.starts_with(*p)) {
            Some(p) => offset += p.chars().count(),
            None => return offset,
        }
    }
}

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}

const VERBS: &[&str] = &[
    "toca", "toque", "tocar", "coloca", "coloque", "bota", "poe", "abre", "abra", "abrir", "pausa", "pause",
    "aumenta", "abaixa", "diminui", "pesquisa", "pesquise", "procura", "busca", "pula", "volta", "manda",
    "envia", "clica", "seleciona", "escreve", "digita", "fecha", "desliga", "cria", "salva",
];

/// "abre o spotify e toca X", "pausa e depois…": more than one action → the brain.
fn is_compound(key: &str) -> bool {
    // "depois de amanhã" is a date, not a second step.
    let key = &key.replace("depois de amanha", "dia2");
    if key.contains(" depois ") || key.contains(" entao ") || key.contains(" e em seguida") {
        return true;
    }
    key.split(" e ").skip(1).any(|part| {
        let first = part.split_whitespace().next().unwrap_or("");
        VERBS.contains(&first)
    })
}

const APPS: &[&str] = &[
    "spotify", "chrome", "google chrome", "edge", "vscode", "vs code", "code", "notepad", "bloco de notas",
    "calc", "calculadora", "explorer", "arquivos", "explorador", "terminal", "powershell", "cmd", "discord",
];

const SITES: &[(&str, &str)] = &[
    ("youtube", "https://www.youtube.com"),
    ("google", "https://www.google.com"),
    ("gmail", "https://mail.google.com"),
    ("whatsapp", "https://web.whatsapp.com"),
    ("whatsapp web", "https://web.whatsapp.com"),
    ("netflix", "https://www.netflix.com"),
    ("github", "https://github.com"),
    ("instagram", "https://www.instagram.com"),
    ("twitter", "https://x.com"),
    ("x", "https://x.com"),
    ("chatgpt", "https://chatgpt.com"),
    ("linkedin", "https://www.linkedin.com"),
    ("twitch", "https://www.twitch.tv"),
    ("mercado livre", "https://www.mercadolivre.com.br"),
];

fn strip_articles(s: &str) -> &str {
    let mut s = s.trim();
    for a in ["o ", "a ", "os ", "as ", "um ", "uma ", "no ", "na "] {
        if let Some(rest) = s.strip_prefix(a) {
            s = rest.trim();
        }
    }
    s
}

fn first_number(key: &str) -> Option<u32> {
    key.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok())
}

pub fn parse(text: &str) -> Option<Intent> {
    let (chars, key_full) = prepare(text);
    if chars.is_empty() || chars.len() > 120 {
        return None;
    }
    let start = skip_polite(&key_full);
    let key: String = key_full.chars().skip(start).collect();
    let orig: String = chars.iter().skip(start).collect();
    let key = key.trim().to_string();
    if key.is_empty() || is_compound(&key) {
        return None;
    }
    let words: Vec<&str> = key.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);

    if let Some(intent) = agenda_intent(&key) {
        return Some(intent);
    }

    // ── "Bom dia" ──
    let norm = words.join(" ");
    if matches!(
        norm.as_str(),
        "bom dia" | "bom dia jarvis" | "boa tarde jarvis" | "boa noite jarvis" | "como esta meu dia" | "como ta meu dia" | "resumo do dia" | "me da o resumo do dia" | "resume meu dia"
    ) {
        return Some(Intent::Briefing);
    }

    // ── Microphone ──
    if key.contains("microfone") || has("mic") {
        if ["desmuta", "desmute", "liga", "ligar", "ativa", "ativar", "volta"].iter().any(|w| key.starts_with(w)) {
            return Some(Intent::Mic(false));
        }
        if ["muta", "mute", "desliga", "desligar", "silencia", "corta", "desativa"].iter().any(|w| key.starts_with(w)) {
            return Some(Intent::Mic(true));
        }
    }

    // ── Audio output ──
    for verb in ["muda pro ", "muda pra ", "muda para o ", "muda para a ", "troca pro ", "troca pra ", "troca para o ", "troca para a ", "som no ", "som na ", "joga o som pro ", "joga o som pra ", "coloca o som no ", "coloca o som na "] {
        if let Some(rest) = key.strip_prefix(verb) {
            let target = strip_articles(rest);
            if ["fone", "fones", "fone de ouvido", "headset", "caixa", "caixa de som", "caixinha", "alto falante", "monitor", "tv", "hdmi"].contains(&target) {
                return Some(Intent::Output(target.to_string()));
            }
        }
    }
    if matches!(key.as_str(), "troca a saida de som" | "troca a saida" | "muda a saida de som" | "troca o som") {
        return Some(Intent::Output(String::new()));
    }

    // ── Volume ──
    if key.contains("volume") || key.starts_with("som ") {
        if has("mudo") || key.contains("muta") || key.contains("silencia") {
            return Some(Intent::Mute);
        }
        if key.contains("maximo") || key.contains("no talo") {
            return Some(Intent::Volume(100));
        }
        if let Some(n) = first_number(&key) {
            if n <= 100 {
                return Some(Intent::Volume(n));
            }
        }
        let step = 10;
        if ["aumenta", "aumente", "sobe", "suba", "mais"].iter().any(|w| has(w)) {
            return Some(Intent::VolumeDelta(step));
        }
        if ["abaixa", "abaixe", "diminui", "diminua", "baixa", "menos"].iter().any(|w| has(w)) {
            return Some(Intent::VolumeDelta(-step));
        }
        return None;
    }
    if matches!(key.as_str(), "muta" | "mudo" | "silencio" | "silencia" | "muta o som") {
        return Some(Intent::Mute);
    }

    // ── Media keys ──
    let media_exact: &[(&[&str], &'static str)] = &[
        (&["pausa", "pause", "pausar", "pausa a musica", "pausa o spotify", "para a musica", "para de tocar", "pausa ai"], "pause"),
        (&["continua", "continuar", "despausa", "retoma", "play", "volta a tocar", "continua a musica", "solta o som", "da play"], "play"),
        (&["proxima", "proxima musica", "pula", "pula essa", "pula a musica", "passa", "passa a musica", "next", "avanca", "proxima faixa"], "next"),
        (&["anterior", "musica anterior", "volta a musica", "volta uma musica", "previous", "faixa anterior"], "previous"),
    ];
    for (phrases, action) in media_exact {
        if phrases.contains(&key.as_str()) {
            return Some(Intent::Media(action));
        }
    }

    // ── "toca X" on Spotify ──
    for verb in ["toca ", "toque ", "tocar ", "coloca ", "coloque ", "bota ", "poe ", "da play em ", "play "] {
        if let Some(_rest) = key.strip_prefix(verb) {
            let q_chars = verb.chars().count();
            let mut query: String = orig.chars().skip(q_chars).collect();
            for tail in [" no spotify", " pra mim", " para mim", " ai"] {
                if let Some(s) = query.trim_end().strip_suffix(tail) {
                    query = s.to_string();
                }
            }
            let query = query.trim().to_string();
            // "toca no youtube…" is a web task, not Spotify.
            if query.contains("youtube") {
                return None;
            }
            let plain: String = query.chars().map(strip_accent).collect();
            let generic = matches!(plain.as_str(), "musica" | "uma musica" | "um som" | "som" | "o som");
            return Some(Intent::SpotifyPlay(if generic { String::new() } else { query }));
        }
    }

    // ── Search the web ──
    for verb in ["pesquisa ", "pesquise ", "procura ", "procure ", "busca ", "busque ", "pesquisa no google ", "busca no google "] {
        if let Some(rest) = key.strip_prefix(verb) {
            let rest = rest.strip_prefix("no google ").unwrap_or(rest).strip_prefix("por ").unwrap_or(rest);
            let q: String = orig.chars().skip(orig.chars().count() - rest.chars().count()).collect();
            if !q.trim().is_empty() {
                return Some(Intent::Search(q.trim().to_string()));
            }
        }
    }

    // ── Open an app or a site ──
    for verb in ["abre ", "abra ", "abrir ", "inicia ", "inicie ", "entra no ", "entra na ", "entrar no ", "vai no ", "vai pro ", "vai para o "] {
        if let Some(rest) = key.strip_prefix(verb) {
            let target = strip_articles(rest);
            if let Some((_, url)) = SITES.iter().find(|(name, _)| *name == target) {
                return Some(Intent::OpenSite(url.to_string()));
            }
            if APPS.contains(&target) {
                return Some(Intent::OpenApp(target.to_string()));
            }
            if !target.contains(' ') && target.contains('.') {
                return Some(Intent::OpenSite(target.to_string()));
            }
            return None;
        }
    }
    None
}

/// Runs an intent. Ok((reply, actions)); Err sends the request to the brain.
pub async fn run(intent: &Intent) -> Result<(String, Vec<String>), String> {
    use crate::jarvis_pc;
    let done = |msg: String| Ok((msg.clone(), vec![msg]));
    match intent {
        Intent::Volume(n) => done(jarvis_pc::jarvis_set_volume(*n)?),
        Intent::VolumeDelta(d) => {
            let now = tokio::task::spawn_blocking(jarvis_pc::get_system_volume).await.ok().flatten().unwrap_or(50) as i32;
            let next = (now + d).clamp(0, 100) as u32;
            done(jarvis_pc::jarvis_set_volume(next)?)
        }
        Intent::Mute => done(jarvis_pc::jarvis_media_control("mute".into())?),
        Intent::Media(action) => {
            // YouTube, VLC… playing: that app, not Spotify.
            let other = tokio::task::spawn_blocking(crate::media_sessions::now).await.ok().flatten();
            if other.is_some_and(|m| m.playing && m.app != "Spotify") {
                let a = (*action).to_string();
                if let Ok(Ok(msg)) = tokio::task::spawn_blocking(move || crate::media_sessions::control(&a)).await {
                    return done(msg);
                }
            }
            if let Some(msg) = crate::jarvis_brain::api_media_control(action).await {
                return done(msg);
            }
            done(jarvis_pc::jarvis_media_control((*action).to_string())?)
        }
        Intent::SpotifyPlay(query) => {
            if !crate::spotify_api::connected() {
                // Without the Web API the old path cannot pick a song reliably: let the brain handle it.
                return Err("spotify api not connected".into());
            }
            let msg = crate::spotify_api::play_query(query, None).await?;
            done(msg)
        }
        Intent::OpenApp(app) => done(jarvis_pc::jarvis_open_target(app.clone())?),
        Intent::OpenSite(url) => {
            let snap = crate::jarvis_browser::open(url, false).await?;
            let title = snap.lines().next().unwrap_or("").trim_start_matches("Página: ").to_string();
            done(format!("Abri {}", if title.is_empty() { url.clone() } else { title }))
        }
        Intent::Search(q) => {
            crate::jarvis_browser::open(q, false).await?;
            done(format!("Pesquisei \"{q}\" no Google."))
        }
        Intent::Briefing => {
            let user = crate::jarvis_store::load_data().user_name;
            let text = crate::jarvis_tools::briefing_speech(&user).await;
            Ok((text, vec!["Resumo do dia".into()]))
        }
        Intent::Mic(mute) => {
            let m = *mute;
            tokio::task::spawn_blocking(move || crate::audio::set_mic_mute(m)).await.map_err(|e| e.to_string())??;
            done(if m { "Microfone mudo.".into() } else { "Microfone ligado.".into() })
        }
        Intent::Output(target) => {
            let t = target.clone();
            done(tokio::task::spawn_blocking(move || crate::audio::switch_output(&t)).await.map_err(|e| e.to_string())??)
        }
        Intent::Agenda(offset, days) => {
            use crate::google_calendar as cal;
            if !cal::connected() {
                return Err("calendar not connected".into());
            }
            let from = cal::today() + chrono::Duration::days(*offset);
            let events = cal::events(from, *days).await?;
            let when = match (offset, days) {
                (0, 1) => "hoje",
                (1, 1) => "amanhã",
                (2, 1) => "depois de amanhã",
                _ => "nos próximos 7 dias",
            };
            let text = if events.is_empty() {
                format!("Você não tem nada marcado {when}.")
            } else {
                format!("{} {when}:\n{}", plural(events.len()), cal::describe(&events, false))
            };
            Ok((text, vec![format!("Agenda {when}")]))
        }
    }
}

fn plural(n: usize) -> String {
    if n == 1 { "1 compromisso".into() } else { format!("{n} compromissos") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_commands() {
        assert_eq!(parse("Volume em 30"), Some(Intent::Volume(30)));
        assert_eq!(parse("Jarvis, coloca o volume pra 75%"), Some(Intent::Volume(75)));
        assert_eq!(parse("aumenta o volume"), Some(Intent::VolumeDelta(10)));
        assert_eq!(parse("abaixa o volume"), Some(Intent::VolumeDelta(-10)));
        assert_eq!(parse("volume no máximo"), Some(Intent::Volume(100)));
    }

    #[test]
    fn new_instant_commands() {
        assert_eq!(parse("Bom dia, Jarvis"), Some(Intent::Briefing));
        assert_eq!(parse("muta o microfone"), Some(Intent::Mic(true)));
        assert_eq!(parse("liga o microfone"), Some(Intent::Mic(false)));
        assert_eq!(parse("muda pro fone"), Some(Intent::Output("fone".into())));
        assert_eq!(parse("joga o som pra caixa de som"), Some(Intent::Output("caixa de som".into())));
        assert_eq!(parse("troca a saída de som"), Some(Intent::Output(String::new())));
    }

    #[test]
    fn media_commands() {
        assert_eq!(parse("Pausa"), Some(Intent::Media("pause")));
        assert_eq!(parse("ei jarvis, próxima música"), Some(Intent::Media("next")));
        assert_eq!(parse("volta a música"), Some(Intent::Media("previous")));
        assert_eq!(parse("continua"), Some(Intent::Media("play")));
    }

    #[test]
    fn spotify_queries_keep_their_accents() {
        assert_eq!(parse("toca Canção da América"), Some(Intent::SpotifyPlay("canção da américa".into())));
        assert_eq!(parse("coloca o álbum Dawn FM no Spotify"), Some(Intent::SpotifyPlay("o álbum dawn fm".into())));
        assert_eq!(parse("toca música"), Some(Intent::SpotifyPlay(String::new())));
    }

    #[test]
    fn apps_and_sites() {
        assert_eq!(parse("abre o YouTube"), Some(Intent::OpenSite("https://www.youtube.com".into())));
        assert_eq!(parse("abre a calculadora"), Some(Intent::OpenApp("calculadora".into())));
        assert_eq!(parse("abre github.com"), Some(Intent::OpenSite("github.com".into())));
        assert_eq!(parse("pesquisa receita de bolo"), Some(Intent::Search("receita de bolo".into())));
    }

    #[test]
    fn agenda_questions_are_instant_and_changes_go_to_the_brain() {
        assert_eq!(parse("o que eu tenho hoje?"), Some(Intent::Agenda(0, 1)));
        assert_eq!(parse("Jarvis, minha agenda de amanhã"), Some(Intent::Agenda(1, 1)));
        assert_eq!(parse("compromissos da semana"), Some(Intent::Agenda(0, 7)));
        assert_eq!(parse("tenho alguma reunião depois de amanhã"), Some(Intent::Agenda(2, 1)));
        assert_eq!(parse("marca reunião amanhã às 15h"), None);
        assert_eq!(parse("cancela a reunião de amanhã"), None);
    }

    #[test]
    fn anything_unclear_or_compound_goes_to_the_brain() {
        assert_eq!(parse("abre o spotify e toca Dawn FM"), None);
        assert_eq!(parse("abre o projeto jarvis"), None);
        assert_eq!(parse("toca no youtube um vídeo de gatos"), None);
        assert_eq!(parse("qual a previsão do tempo amanhã?"), None);
        assert_eq!(parse("entra no youtube e clica no primeiro vídeo"), None);
    }
}
