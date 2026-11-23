//! Jarvis AI Brain: Groq + Gemini automatic free fallback, Ollama local support,
//! Claude/Gemini CLI & OAuth account support, Groq Whisper STT, and confirmed PC actions.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

use crate::jarvis_accounts::{self, AiAccount};
use crate::jarvis_config;
use crate::jarvis_pc;
use crate::jarvis_store;
use crate::secrets;
use crate::Shared;

const GROQ_CHAT_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const GROQ_WHISPER_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
/// Groq retires models from time to time; the first of these the account has
/// is used (the list is read again when a model stops existing).
const GROQ_MODELS: &[&str] = &[
    "openai/gpt-oss-120b",
    "moonshotai/kimi-k2-instruct-0905",
    "moonshotai/kimi-k2-instruct",
    "llama-3.3-70b-versatile",
    "meta-llama/llama-4-maverick-17b-128e-instruct",
    "qwen/qwen3-32b",
    "openai/gpt-oss-20b",
    "llama-3.1-8b-instant",
];
static GROQ_PICKED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The Groq chat model to use with this key (cached).
async fn groq_model(api_key: &str, refresh: bool) -> String {
    if !refresh {
        if let Some(m) = GROQ_PICKED.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return m;
        }
    }
    let listed: Vec<String> = match http_client() {
        Ok(c) => match c.get("https://api.groq.com/openai/v1/models").bearer_auth(api_key).send().await {
            Ok(r) => r
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v["data"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let pick = GROQ_MODELS
        .iter()
        .find(|m| listed.iter().any(|l| l == *m))
        .map(|m| m.to_string())
        .or_else(|| {
            // None of ours: any chat model (not speech, not a guard model).
            listed.iter().find(|l| !l.contains("whisper") && !l.contains("tts") && !l.contains("guard") && !l.contains("orpheus")).cloned()
        })
        .unwrap_or_else(|| GROQ_MODELS[0].to_string());
    crate::log::line(format!("groq: usando o modelo {pick}"));
    *GROQ_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(pick.clone());
    pick
}
/// The full large-v3, not turbo: more accurate in Portuguese (turbo slipped
/// into Spanish and missed names), and still well under a second on Groq.
const GROQ_WHISPER_MODEL: &str = "whisper-large-v3";

/// Words the transcription often gets wrong: apps, services, the assistant's
/// name. Whisper takes them as context (its `prompt`), Gemini as a hint.
const STT_VOCAB: &str = "Jarvis, Spotify, YouTube, WhatsApp, OpenBot, Obsidian, Claude, Gemini, ChatGPT, GitHub, Google Agenda, Gmail, Steam, Discord, Chrome, FC 26";

/// The vocabulary for the transcription: the built-in words plus the user's own
/// (Ajustes → Microfone → Palavras que o Jarvis deve conhecer).
pub fn stt_vocabulary() -> String {
    let own = crate::settings::load().jarvis_voice_vocab;
    let mut words: Vec<String> = Vec::new();
    for w in own.split([',', ';', '\n']).chain(STT_VOCAB.split(',')) {
        let w = w.trim();
        if !w.is_empty() && !words.iter().any(|x| x.eq_ignore_ascii_case(w)) {
            words.push(w.to_string());
        }
    }
    // Whisper reads at most ~224 tokens of prompt; keep it well below.
    let mut out = String::new();
    for w in words {
        if out.len() + w.len() > 600 {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&w);
    }
    out
}

/// On silence Whisper sometimes "transcribes" its own prompt: that is nothing.
fn drop_prompt_echo(text: &str) -> String {
    let t = text.trim();
    let lower = t.to_lowercase();
    if lower.contains("comandos de voz para o assistente") || lower.contains(&STT_VOCAB[..40].to_lowercase()) {
        return String::new();
    }
    t.to_string()
}

/// Whisper's prompt: a sentence in the style of the commands, then the words.
fn whisper_prompt() -> String {
    format!("Comandos de voz para o assistente Jarvis, em português do Brasil. {}.", stt_vocabulary())
}
const GEMINI_MODEL: &str = "gemini-3.8-flash";
const GEMINI_MODELS: &[&str] = &[GEMINI_MODEL, "gemini-3.5-flash-lite"];
const OPENAI_CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_MODEL: &str = "gpt-4o-mini";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_MODEL: &str = "claude-sonnet-5-5";

/// Total time the agentic loop may spend on one request (all model calls and
/// tools), so a voice command never hangs for minutes.
const AGENT_BUDGET: Duration = Duration::from_secs(45);
/// Multi-step web tasks (open, find, click, check) need more room.
const BROWSER_BUDGET: Duration = Duration::from_secs(150);

static PENDING_ACTIONS: LazyLock<Mutex<HashMap<String, PendingAction>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACTION_SEQ: AtomicU64 = AtomicU64::new(0);
/// Set once a Gemini model rejects `thinkingConfig`, so it is not sent again.
static THINKING_UNSUPPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A rough risk label for the confirmation card.
fn risk_level(command: &str) -> &'static str {
    let lower = command.to_lowercase();
    const DANGER: &[&str] = &[
        "remove-item", "rm ", "del ", "rd ", "rmdir", "format", "stop-computer", "restart-computer",
        "shutdown", "reg ", "set-itemproperty", "remove-itemproperty", "set-executionpolicy", "bcdedit",
        "diskpart", "cipher", "takeown", "icacls", "net user", "invoke-webrequest", "iwr ", "invoke-expression",
        "iex ", "start-bitstransfer", "stop-process", "taskkill", "uninstall",
    ];
    if DANGER.iter().any(|d| lower.contains(d)) {
        "danger"
    } else {
        "warning"
    }
}

/// Holds a system command until the user taps Permitir/Cancelar in the island.
fn queue_pending(kind: &str, command: &str, description: &str) -> PendingAction {
    let seq = ACTION_SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let action = PendingAction {
        id: format!("act_{millis}_{seq}"),
        kind: kind.to_string(),
        command: command.to_string(),
        description: if description.trim().is_empty() {
            format!("Executar {kind}")
        } else {
            description.trim().to_string()
        },
        risk_level: risk_level(command).to_string(),
    };
    PENDING_ACTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(action.id.clone(), action.clone());
    action
}

const AWAITING_CONFIRMATION: &str =
    "Aguardando o usuário tocar em Permitir ou Cancelar na ilha. Não execute de novo; avise que o comando está esperando confirmação.";

const JARVIS_SYSTEM_PROMPT: &str = r#"Você é o Jarvis, o assistente pessoal inteligente do usuário integrado ao Windows.
Converse de forma natural, solta, inteligente e proativa em português do Brasil — nunca robótica, seca ou repetitiva.

Regras de ouro de comportamento e precisão:
1. Você TEM permissão direta (Administrador) para executar ações no Windows do usuário. NUNCA devolva blocos de código PowerShell pedindo para o usuário copiar, rodar ou clicar manualmente — execute você mesmo usando suas ferramentas.
2. Quando ferramentas (function calling) estiverem disponíveis, SEMPRE execute primeiro as ferramentas necessárias, observe o retorno real de cada uma, encadeie novas etapas se o pedido for composto ou precisar de ajuste, e só responda ao usuário depois de concluir e conferir os resultados.
3. NUNCA invente que uma ação deu certo se a ferramenta retornou erro ou aviso de falha. Seja honesto sobre o que funcionou e tente corrigir antes de responder.
4. Use o "Contexto ao vivo do PC" abaixo! Exemplos:
   - Se o usuário disser "toca outro álbum desse artista" ou "muda de álbum", olhe qual artista/álbum está no contexto atual do Spotify, escolha um álbum DIFERENTE desse mesmo artista pelo nome completo e chame `spotify_play(query="...", kind="album")`.
   - Se precisar inspecionar processos rodando ou o conteúdo da área de transferência antes de agir, chame `get_pc_state(include_processes=true, include_clipboard=true)`.
   - Para ajustar volume exato (ex: 50%), chame `set_volume(percent=50)` (NUNCA PowerShell).
   - Para tocar música/álbum/artista/playlist específica no Spotify, chame `spotify_play` (NUNCA `media_control(play)` sozinho, pois `media_control` só retoma a faixa antiga).
   - Para tarefas de desenvolvimento em projetos (código, build, API, design, memória Obsidian), delegue com `delegate_openbot`.
5. Navegador: para QUALQUER site use `browser_open` (nunca `open_target`). Para interagir, use os números [n] do snapshot mais recente com `browser_click`, `browser_type`, `browser_select`, `browser_press`, `browser_scroll`. Cada ação devolve um snapshot novo: CONFIRME nele que a ação teve efeito (página mudou, campo com o valor, item marcado/selecionado) antes de dizer que fez. Se não mudou, tente outro elemento ou outra abordagem; se não conseguir, diga claramente o que faltou. Nunca diga "selecionei"/"cliquei" sem essa confirmação.
6. Seja rápido e curto: chame as ferramentas direto, sem explicar o plano antes. A resposta final tem 1 ou 2 frases.
7. Agenda (Google Agenda): "me lembra / me avisa / notifica" vira evento com `remind_minutes` (o Google notifica no celular e o Jarvis avisa no PC). Use `calendar_list`, `calendar_create`, `calendar_update`, `calendar_delete`. Datas/horas sempre no horário LOCAL no formato AAAA-MM-DDTHH:MM, calculadas a partir de "Agora" abaixo ("amanhã às 15h", "sexta que vem"). Sem duração dita, use 1 hora. Para mudar ou apagar, chame `calendar_list` antes para pegar o id. Só apague se o usuário pediu claramente para apagar/cancelar aquele evento."#;

const JARVIS_SYSTEM_PROMPT_TAGS: &str = r#"

Caso esteja rodando em um provedor sem function calling nativo, você ainda pode emitir tags ao final da resposta:
[[ACTION:spotify_play|termo]], [[ACTION:volume|N]], [[ACTION:media|play|pause|play_pause|next|prev|mute]], [[ACTION:open|alvo]], [[ACTION:powershell|cmd]], [[ACTION:pc_exec|tipo|cmd]], [[ACTION:memory|...]], [[ACTION:task|...]], [[ACTION:reminder|título|HH:MM]], [[ACTION:timer|min|nome]], [[ACTION:routine|Nome|Desc|alvos]], [[ACTION:run_routine|Nome]], [[ACTION:openbot|agente|comando]], [[ACTION:tab|aba]]."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAction {
    pub id: String,
    pub kind: String,
    pub command: String,
    pub description: String,
    pub risk_level: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionExecutionResult {
    pub id: String,
    pub executed: bool,
    pub success: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainStatus {
    pub groq_configured: bool,
    pub gemini_configured: bool,
    pub openai_configured: bool,
    pub claude_configured: bool,
    pub ollama_url: String,
    pub brain_mode: String,
    pub claude_cli_installed: bool,
    pub claude_cli_logged_in: bool,
    pub gemini_cli_installed: bool,
    pub gemini_cli_logged_in: bool,
    pub claude_oauth_configured: bool,
    pub gemini_oauth_configured: bool,
    pub available_providers: Vec<String>,
    pub available_agents: Vec<AgentOption>,
    pub accounts: Vec<AiAccount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarvisReply {
    pub text: String,
    pub provider_used: String,
    pub actions: Vec<String>,
    pub pending_actions: Vec<PendingAction>,
}

fn available_agents_list() -> Vec<AgentOption> {
    vec![
        AgentOption {
            id: "orquestrador".into(),
            label: "Orquestrador".into(),
            description: "Coordena demandas, planeja e delega entre especialistas".into(),
        },
        AgentOption {
            id: "frontend-mobile".into(),
            label: "Frontend & Mobile".into(),
            description: "React, Next.js, TypeScript, Vite, Tailwind e React Native".into(),
        },
        AgentOption {
            id: "backend-dados".into(),
            label: "Backend & Dados".into(),
            description: "APIs, Python, Node/TS, Rust, Supabase, PostgreSQL e SQLite".into(),
        },
        AgentOption {
            id: "devops".into(),
            label: "DevOps & Infra".into(),
            description: "Docker, CI/CD, deploy, servidores, Git e automação".into(),
        },
        AgentOption {
            id: "ui-ux".into(),
            label: "UI/UX & Design".into(),
            description: "Interface, usabilidade, CSS, animações e design system".into(),
        },
    ]
}

fn is_known_agent(s: &str) -> bool {
    matches!(
        s.trim().to_lowercase().as_str(),
        "orquestrador" | "frontend-mobile" | "backend-dados" | "devops" | "ui-ux"
    )
}

fn normalize_agent(agent: Option<String>) -> Option<String> {
    let raw = agent?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    let mapped = match lower.as_str() {
        "orquestrador" | "orquestrador geral" => "orquestrador",
        "frontend" | "frontend-mobile" | "frontend & mobile" | "mobile" => "frontend-mobile",
        "backend" | "backend-dados" | "backend & dados" | "dados" => "backend-dados",
        "devops" | "infra" | "devops & infra" => "devops",
        "ui" | "ux" | "ui-ux" | "ui/ux" | "design" => "ui-ux",
        _ => return Some(trimmed.to_string()),
    };
    Some(mapped.to_string())
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn find_cli_binary(name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("where.exe")
            .arg(name)
            .creation_flags(0x0800_0000)
            .output()
        {
            if out.status.success() {
                if let Some(first) = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                {
                    return Some(PathBuf::from(first));
                }
            }
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let npm_cmd = PathBuf::from(appdata).join("npm").join(format!("{name}.cmd"));
            if npm_cmd.exists() {
                return Some(npm_cmd);
            }
        }
        if let Some(home) = user_home_dir() {
            let local_exe = home.join(".local").join("bin").join(format!("{name}.exe"));
            if local_exe.exists() {
                return Some(local_exe);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        if let Ok(out) = std::process::Command::new("which").arg(name).output() {
            if out.status.success() {
                if let Some(first) = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                {
                    return Some(PathBuf::from(first));
                }
            }
        }
        None
    }
}

fn read_claude_oauth_token() -> Option<String> {
    if let Some(tok) = secrets::get("claude-oauth-token") {
        return Some(tok);
    }
    let home = user_home_dir()?;
    let creds = home.join(".claude").join(".credentials.json");
    let raw = std::fs::read_to_string(creds).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v["claudeAiOauth"]["accessToken"]
        .as_str()
        .or_else(|| v["accessToken"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn read_gemini_oauth_token() -> Option<String> {
    if let Some(tok) = secrets::get("gemini-oauth-token") {
        return Some(tok);
    }
    let home = user_home_dir()?;
    let creds = home.join(".gemini").join("oauth_creds.json");
    let raw = std::fs::read_to_string(creds).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v["access_token"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn gemini_oauth_file_present() -> bool {
    let Some(home) = user_home_dir() else {
        return false;
    };
    home.join(".gemini").join("oauth_creds.json").exists()
}

#[tauri::command]
pub fn jarvis_brain_status(shared: State<Shared>) -> BrainStatus {
    let s = shared.settings.lock().unwrap();
    let groq_configured = jarvis_accounts::get_active_credentials("groq").is_some();
    let gemini_configured = jarvis_accounts::get_active_credentials("gemini").is_some();
    let openai_configured = jarvis_accounts::get_active_credentials("openai").is_some();
    let claude_cli_installed = find_cli_binary("claude").is_some();
    let gemini_cli_installed = find_cli_binary("gemini").is_some();
    let claude_oauth_configured =
        jarvis_accounts::get_active_credentials("claude").is_some() || read_claude_oauth_token().is_some();
    let gemini_oauth_configured = read_gemini_oauth_token().is_some() || gemini_oauth_file_present();
    let claude_configured = claude_oauth_configured || claude_cli_installed;
    let claude_cli_logged_in = claude_cli_installed && claude_oauth_configured;
    let gemini_cli_logged_in = gemini_cli_installed && gemini_oauth_configured;

    let mut available_providers = vec!["auto".to_string()];
    if groq_configured {
        available_providers.push("groq".into());
    }
    if gemini_configured {
        available_providers.push("gemini".into());
    }
    if openai_configured {
        available_providers.push("openai".into());
    }
    if claude_oauth_configured {
        available_providers.push("claude".into());
    }
    if claude_cli_installed {
        available_providers.push("claude-cli".into());
    }
    if gemini_cli_installed {
        available_providers.push("gemini-cli".into());
    }
    if claude_oauth_configured {
        available_providers.push("claude-oauth".into());
    }
    if gemini_oauth_configured {
        available_providers.push("gemini-oauth".into());
    }
    available_providers.push("ollama".into());

    BrainStatus {
        groq_configured,
        gemini_configured,
        openai_configured,
        claude_configured,
        ollama_url: s.ollama_url.clone(),
        brain_mode: s.jarvis_brain_mode.clone(),
        claude_cli_installed,
        claude_cli_logged_in,
        gemini_cli_installed,
        gemini_cli_logged_in,
        claude_oauth_configured,
        gemini_oauth_configured,
        available_providers,
        available_agents: available_agents_list(),
        accounts: jarvis_accounts::list_all_accounts(),
    }
}

#[tauri::command]
pub async fn jarvis_execute_confirmed_action(
    action_id: String,
    approved: bool,
) -> Result<ActionExecutionResult, String> {
    let action = {
        let mut map = PENDING_ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&action_id)
    }
    .ok_or_else(|| "Ação pendente não encontrada ou já processada.".to_string())?;

    if !approved {
        return Ok(ActionExecutionResult {
            id: action.id,
            executed: false,
            success: true,
            output: "Ação recusada pelo usuário.".into(),
        });
    }

    let kind = action.kind.clone();
    let command = action.command.clone();
    let res = match crate::jarvis_tools::run_confirmed(&kind, &command).await {
        Some(r) => r,
        None => tokio::task::spawn_blocking(move || jarvis_pc::execute_pc_action(&kind, &command))
            .await
            .map_err(|e| format!("Falha ao executar tarefa: {e}"))?,
    };

    match res {
        Ok(output) => Ok(ActionExecutionResult {
            id: action.id,
            executed: true,
            success: true,
            output,
        }),
        Err(err) => Ok(ActionExecutionResult {
            id: action.id,
            executed: true,
            success: false,
            output: err,
        }),
    }
}

#[tauri::command]
pub fn jarvis_list_pending_actions() -> Vec<PendingAction> {
    PENDING_ACTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}

#[tauri::command]
pub fn jarvis_cli_login(provider: String) -> Result<String, String> {
    let clean = provider.trim().to_lowercase();
    let (bin_name, args): (&str, Vec<&str>) = match clean.as_str() {
        "claude" | "claude-cli" => ("claude", vec!["auth", "login"]),
        "gemini" | "gemini-cli" => ("gemini", vec![]),
        _ => return Err(format!("Provedor CLI desconhecido: {provider}")),
    };
    let bin = find_cli_binary(bin_name).ok_or_else(|| {
        format!("CLI '{bin_name}' não encontrado no PATH. Instale primeiro para fazer login.")
    })?;

    #[cfg(windows)]
    {
        let bin_str = bin.to_string_lossy().to_string();
        let cmdline = if args.is_empty() {
            format!("& '{bin_str}'")
        } else {
            format!("& '{bin_str}' {}", args.join(" "))
        };
        std::process::Command::new("cmd")
            .args(["/C", "start", "powershell", "-NoExit", "-Command", &cmdline])
            .spawn()
            .map_err(|e| format!("Falha ao abrir terminal de login: {e}"))?;
        Ok(format!("Terminal aberto para login no {bin_name}."))
    }
    #[cfg(not(windows))]
    {
        let _ = (bin, args);
        Err("Login interativo via terminal implementado para Windows".into())
    }
}

#[tauri::command]
pub fn jarvis_save_account_token(provider: String, token: String) -> Result<(), String> {
    let key = match provider.trim().to_lowercase().as_str() {
        "claude" | "claude-oauth" => "claude-oauth-token",
        "gemini" | "gemini-oauth" => "gemini-oauth-token",
        _ => return Err(format!("Provedor de conta desconhecido: {provider}")),
    };
    secrets::set(key, token.trim())
}

#[tauri::command]
pub async fn jarvis_send_to_openbot(
    command: String,
    project: Option<String>,
    agent: Option<String>,
) -> Result<String, String> {
    let norm_agent = normalize_agent(agent);
    send_to_openbot_webhook(command, project, norm_agent).await
}

fn emit_status(app: &AppHandle, phase: &str, title: &str, detail: &str) {
    let _ = app.emit_to(
        crate::island::WINDOW_LABEL,
        "jarvis-status",
        json!({
            "phase": phase,
            "title": title,
            "detail": detail,
        }),
    );
}

#[tauri::command]
pub async fn jarvis_ask(
    app: AppHandle,
    shared: State<'_, Shared>,
    prompt: String,
    history: Vec<BrainMessage>,
    mode_override: Option<String>,
    image_base64: Option<String>,
    agent: Option<String>,
) -> Result<JarvisReply, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("Mensagem vazia".into());
    }

    emit_status(&app, "thinking", "Jarvis está pensando…", "");
    let res = jarvis_ask_inner(
        &app,
        &shared,
        prompt,
        history,
        mode_override,
        image_base64,
        agent,
    )
    .await;
    emit_status(&app, "idle", "", "");
    res
}

async fn jarvis_ask_inner(
    app: &AppHandle,
    shared: &State<'_, Shared>,
    prompt: String,
    history: Vec<BrainMessage>,
    mode_override: Option<String>,
    image_base64: Option<String>,
    agent: Option<String>,
) -> Result<JarvisReply, String> {
    let selected_agent = normalize_agent(agent);

    // "Sim" / "não" to "Posso enviar?" (a WhatsApp message read back to the
    // user): answered here, at once, without the AI.
    if let Some(text) = crate::whatsapp::answer_held(&prompt).await {
        return Ok(JarvisReply { text, provider_used: "WhatsApp".to_string(), actions: Vec::new(), pending_actions: Vec::new() });
    }
    // "Liga para X (no WhatsApp)": asked back at once, no AI round trip.
    if let Some(text) = crate::whatsapp::quick_call(&prompt) {
        return Ok(JarvisReply { text, provider_used: "WhatsApp".to_string(), actions: Vec::new(), pending_actions: Vec::new() });
    }

    // Simple, unambiguous commands run right away, with no AI round trip.
    if image_base64.as_deref().map_or(true, str::is_empty) {
        if let Some(intent) = crate::jarvis_fastpath::parse(&prompt) {
            emit_status(app, "executing", "Executando…", &prompt);
            match crate::jarvis_fastpath::run(&intent).await {
                Ok((text, actions)) => {
                    return Ok(JarvisReply {
                        text,
                        provider_used: "Atalho".to_string(),
                        actions,
                        pending_actions: Vec::new(),
                    })
                }
                Err(e) => crate::log::line(format!("fastpath {intent:?} -> brain: {e}")),
            }
        }
    }

    let (mode, ollama_url, failover) = {
        let s = shared.settings.lock().unwrap();
        let m = mode_override
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| s.jarvis_brain_mode.clone());
        (m, s.ollama_url.clone(), s.jarvis_ai_failover)
    };

    let data = jarvis_store::load_data();
    let mut system_prompt = format!(
        "{JARVIS_SYSTEM_PROMPT}{}\nNome do usuário: {}.\nAgora: {}.",
        crate::jarvis_tools::PROMPT_RULES,
        data.user_name,
        crate::google_calendar::now_context()
    );
    if !crate::google_calendar::connected() {
        system_prompt.push_str(" Google Agenda NÃO conectado: se pedirem agenda, diga para conectar na aba Agenda.");
    }
    if let Some(ref ag) = selected_agent {
        system_prompt.push_str(&format!(
            "\nAgente selecionado para esta tarefa: {ag}. Se precisar delegar via `delegate_openbot`, direcione para este agente."
        ));
    }

    let live_snapshot = tokio::task::spawn_blocking(|| jarvis_pc::get_live_pc_snapshot(false, false))
        .await
        .unwrap_or_default();
    if !live_snapshot.is_empty() {
        system_prompt.push_str("\nContexto ao vivo do PC:\n");
        system_prompt.push_str(&live_snapshot);
    }

    if !data.memories.is_empty() {
        let mems: Vec<String> = data
            .memories
            .iter()
            .filter(|m| m.category != "user")
            .take(40)
            .map(|m| {
                if m.notes.is_empty() {
                    format!("- [{}] {}", m.category, m.label)
                } else {
                    format!("- [{}] {}: {}", m.category, m.label, m.notes.replace('\n', " | "))
                }
            })
            .collect();
        if !mems.is_empty() {
            system_prompt.push_str("\nMalha Neural (Projetos e Memórias do usuário):\n");
            system_prompt.push_str(&mems.join("\n"));
        }
    }
    if !data.routines.is_empty() {
        let rots: Vec<String> = data
            .routines
            .iter()
            .map(|r| format!("- {}: {} (alvos: {})", r.name, r.description, r.targets.join(", ")))
            .collect();
        system_prompt.push_str("\nComandos / Rotinas personalizadas do usuário:\n");
        system_prompt.push_str(&rots.join("\n"));
    }

    let obsidian_ctx = jarvis_store::load_obsidian_context(&prompt);
    if !obsidian_ctx.is_empty() {
        system_prompt.push_str("\nMemória Unificada (Obsidian):\n");
        system_prompt.push_str(&obsidian_ctx);
    }

    let groq_creds = jarvis_accounts::get_active_credentials("groq");
    let gemini_creds = jarvis_accounts::get_active_credentials("gemini");
    let openai_creds = jarvis_accounts::get_active_credentials("openai");
    let claude_creds = jarvis_accounts::get_active_credentials("claude")
        .map(|(t, _)| t)
        .or_else(read_claude_oauth_token);
    let gemini_oauth = gemini_creds
        .as_ref()
        .filter(|(_, m)| m == "oauth")
        .map(|(t, _)| t.clone())
        .or_else(read_gemini_oauth_token);
    let gemini_key = gemini_creds
        .as_ref()
        .filter(|(_, m)| m != "oauth")
        .map(|(t, _)| t.clone());
    let creds = Creds {
        groq: groq_creds.map(|(t, _)| t),
        gemini_key,
        gemini_oauth,
        openai: openai_creds.map(|(t, _)| t),
        claude: claude_creds,
        ollama_url: if ollama_url.trim().is_empty() {
            "http://localhost:11434".to_string()
        } else {
            ollama_url
        },
    };
    let image = image_base64.as_deref().filter(|s| !s.is_empty());
    let (claude_cli, gemini_cli) = (find_cli_binary("claude").is_some(), find_cli_binary("gemini").is_some());
    let primary = plan_routes(&mode, &creds, image.is_some(), claude_cli, gemini_cli);
    let routes: Vec<(Route, Option<String>)> = if failover {
        let auto = plan_routes("auto", &creds, image.is_some(), claude_cli, gemini_cli).unwrap_or_default();
        let plan = failover_plan(primary.as_deref().unwrap_or(&[]), &auto, &alternate_accounts(image.is_some()));
        if plan.is_empty() {
            return Err(primary.err().unwrap_or_else(|| "Nenhuma IA conectada.".into()));
        }
        plan
    } else {
        primary?.into_iter().map(|r| (r, None)).collect()
    };

    let sa = selected_agent.as_deref();
    let mut errors: Vec<String> = Vec::new();
    for (i, (route, account)) in routes.iter().enumerate() {
        if i > 0 {
            emit_status(app, "thinking", &format!("Trocando para {}…", route.label()), errors.last().map(String::as_str).unwrap_or(""));
        }
        let tools_before = TOOL_RUNS.load(Ordering::Relaxed);
        match run_route(app, route, &system_prompt, &prompt, &history, image, sa).await {
            Ok(mut reply) => {
                if !errors.is_empty() {
                    crate::log::line(format!(
                        "jarvis: fallback to {} after: {}",
                        reply.provider_used,
                        errors.join(" | ")
                    ));
                    reply.provider_used = format!("{} · reserva", reply.provider_used);
                    // Another key of the same AI answered: keep using it.
                    if let Some(id) = account {
                        if let Ok(list) = jarvis_accounts::activate(id) {
                            let _ = app.emit("jarvis-accounts-updated", &list);
                        }
                    }
                }
                return Ok(reply);
            }
            Err(err) => {
                cool_down(route, &err);
                errors.push(format!("{}: {err}", route.label()));
                // It failed after doing something (an event created, a song
                // played…): another AI would do it again.
                if TOOL_RUNS.load(Ordering::Relaxed) != tools_before {
                    break;
                }
            }
        }
    }
    Err(if errors.len() == 1 {
        errors.remove(0)
    } else {
        format!("Nenhuma IA respondeu. {}", errors.join(" · "))
    })
}

/// Native tools run so far (failover never repeats a request that already acted).
static TOOL_RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Routes that failed recently, tried last until this time.
static COOLDOWN: std::sync::LazyLock<Mutex<HashMap<String, std::time::Instant>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn route_key(r: &Route) -> String {
    let tail = |t: &str| t.chars().rev().take(6).collect::<String>();
    match r {
        Route::GeminiKey(t) | Route::GeminiOAuth(t) | Route::Groq(t) | Route::OpenAi(t) | Route::Claude(t) => format!("{}:{}", r.label(), tail(t)),
        Route::Ollama(u) => format!("Ollama:{u}"),
        other => other.label().to_string(),
    }
}

/// Out of quota or rejected keys rest 15 min; a network blip or a 5xx, 2 min.
fn cool_down(r: &Route, err: &str) {
    let e = err.to_lowercase();
    let quota = ["429", "quota", "rate limit", "resource_exhausted", "401", "403", "invalid", "inválid", "expired", "expirad", "credit", "billing", "insufficient"]
        .iter()
        .any(|k| e.contains(k));
    let secs = if quota { 15 * 60 } else { 120 };
    if let Ok(mut m) = COOLDOWN.lock() {
        m.insert(route_key(r), std::time::Instant::now() + std::time::Duration::from_secs(secs));
    }
}

fn cooling(r: &Route) -> bool {
    COOLDOWN.lock().ok().and_then(|m| m.get(&route_key(r)).copied()).is_some_and(|until| std::time::Instant::now() < until)
}

/// The other saved keys of each AI (route, account id), for failover.
fn alternate_accounts(has_image: bool) -> Vec<(Route, Option<String>)> {
    let mut out = Vec::new();
    for provider in ["gemini", "groq", "openai", "claude"] {
        if has_image && !matches!(provider, "gemini" | "openai") {
            continue;
        }
        for (acc, token) in jarvis_accounts::usable_accounts(provider) {
            let route = match provider {
                "gemini" => Route::GeminiKey(token),
                "groq" => Route::Groq(token),
                "openai" => Route::OpenAi(token),
                _ => Route::Claude(token),
            };
            out.push((route, Some(acc.id)));
        }
    }
    out
}

/// The chosen AI first, then its other saved keys, then the other AIs (each
/// with its keys). Routes that failed recently go to the end.
fn failover_plan(primary: &[Route], auto: &[Route], alternates: &[(Route, Option<String>)]) -> Vec<(Route, Option<String>)> {
    let mut plan: Vec<(Route, Option<String>)> = Vec::new();
    let push = |plan: &mut Vec<(Route, Option<String>)>, r: &Route, id: Option<String>| {
        if !plan.iter().any(|(x, _)| x == r) {
            plan.push((r.clone(), id));
        }
    };
    let same_kind = |a: &Route, b: &Route| std::mem::discriminant(a) == std::mem::discriminant(b);
    for r in primary {
        push(&mut plan, r, None);
    }
    for (r, id) in alternates {
        if primary.iter().any(|p| same_kind(p, r)) {
            push(&mut plan, r, id.clone());
        }
    }
    for r in auto {
        push(&mut plan, r, None);
        for (a, id) in alternates {
            if same_kind(a, r) {
                push(&mut plan, a, id.clone());
            }
        }
    }
    // Keep the order, but what is resting goes last (still tried if all else fails).
    let (ready, resting): (Vec<_>, Vec<_>) = plan.into_iter().partition(|(r, _)| !cooling(r));
    ready.into_iter().chain(resting).collect()
}

/// Credentials available for one request.
struct Creds {
    groq: Option<String>,
    gemini_key: Option<String>,
    gemini_oauth: Option<String>,
    openai: Option<String>,
    claude: Option<String>,
    ollama_url: String,
}

/// One way to answer a request.
#[derive(Debug, Clone, PartialEq)]
enum Route {
    GeminiKey(String),
    GeminiOAuth(String),
    Groq(String),
    OpenAi(String),
    Claude(String),
    ClaudeCli,
    GeminiCli,
    Ollama(String),
}

impl Route {
    fn label(&self) -> &'static str {
        match self {
            Route::GeminiKey(_) => "Gemini",
            Route::GeminiOAuth(_) => "Gemini (Conta)",
            Route::Groq(_) => "Groq",
            Route::OpenAi(_) => "OpenAI",
            Route::Claude(_) => "Claude",
            Route::ClaudeCli => "Claude CLI",
            Route::GeminiCli => "Gemini CLI",
            Route::Ollama(_) => "Ollama",
        }
    }
}

/// The ordered routes for a brain mode. A specific mode gives its route(s) or an
/// error saying what to connect; "auto" gives every connected provider, tried
/// in order until one answers. With an image only vision-capable routes count.
fn plan_routes(
    mode: &str,
    c: &Creds,
    has_image: bool,
    claude_cli: bool,
    gemini_cli: bool,
) -> Result<Vec<Route>, String> {
    let gemini_routes = || {
        let mut v = Vec::new();
        if let Some(k) = &c.gemini_key {
            v.push(Route::GeminiKey(k.clone()));
        }
        if let Some(t) = &c.gemini_oauth {
            v.push(Route::GeminiOAuth(t.clone()));
        }
        v
    };

    if has_image {
        let mut v = gemini_routes();
        if let Some(k) = &c.openai {
            v.push(Route::OpenAi(k.clone()));
        }
        if v.is_empty() {
            return Err(
                "Para ler a tela, conecte uma conta Google Gemini ou OpenAI em Configurações → Cérebro.".to_string(),
            );
        }
        return Ok(v);
    }

    let need = |r: Option<Route>, msg: &str| r.map(|r| vec![r]).ok_or_else(|| msg.to_string());
    match mode {
        "gemini" => {
            let v = gemini_routes();
            if v.is_empty() {
                Err("Conta Gemini não conectada. Clique em Entrar no Gemini em Configurações → Cérebro.".to_string())
            } else {
                Ok(v)
            }
        }
        "gemini-oauth" => need(
            c.gemini_oauth.clone().map(Route::GeminiOAuth),
            "Conta Gemini OAuth não conectada. Clique em Entrar no Gemini em Configurações → Cérebro.",
        ),
        "groq" => need(
            c.groq.clone().map(Route::Groq),
            "Conta Groq não conectada. Clique em Entrar no Groq em Configurações → Cérebro.",
        ),
        "openai" | "chatgpt" => need(
            c.openai.clone().map(Route::OpenAi),
            "Conta OpenAI / ChatGPT não conectada. Clique em Entrar no ChatGPT em Configurações → Cérebro.",
        ),
        "claude" | "claude-oauth" => need(
            c.claude.clone().map(Route::Claude),
            "Conta Claude não conectada. Clique em Entrar no Claude em Configurações → Cérebro.",
        ),
        "claude-cli" => Ok(vec![Route::ClaudeCli]),
        "gemini-cli" => Ok(vec![Route::GeminiCli]),
        "ollama" => Ok(vec![Route::Ollama(c.ollama_url.clone())]),
        _ => {
            let mut v = gemini_routes();
            if let Some(k) = &c.groq {
                v.push(Route::Groq(k.clone()));
            }
            if let Some(k) = &c.openai {
                v.push(Route::OpenAi(k.clone()));
            }
            if let Some(k) = &c.claude {
                v.push(Route::Claude(k.clone()));
            }
            if claude_cli {
                v.push(Route::ClaudeCli);
            }
            if gemini_cli {
                v.push(Route::GeminiCli);
            }
            if v.is_empty() {
                Err("Nenhuma conta conectada. Abra Configurações → Cérebro e clique em Entrar em uma das IAs.".into())
            } else {
                Ok(v)
            }
        }
    }
}

async fn run_route(
    app: &AppHandle,
    route: &Route,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
    image: Option<&str>,
    sa: Option<&str>,
) -> Result<JarvisReply, String> {
    let (auth, provider) = match route {
        Route::GeminiKey(k) => (
            Some(GeminiAuth::ApiKey(k.clone())),
            if image.is_some() { "Google Gemini Vision" } else { "Google Gemini" },
        ),
        Route::GeminiOAuth(t) => (
            Some(GeminiAuth::OAuth(t.clone())),
            if image.is_some() { "Google Gemini Vision (Conta)" } else { "Gemini (Conta)" },
        ),
        _ => (None, ""),
    };
    if let Some(auth) = auth {
        let (text, actions, pending_actions) = match call_gemini_agentic(app, &auth, system_prompt, prompt, history, image, sa).await {
            // Google's filter refused the request — most often for something
            // earlier in the conversation: once more with this message alone.
            Err(e) if e.contains("pedido bloqueado") && !history.is_empty() => {
                crate::log::line("gemini: pedido bloqueado pelo filtro; tentando sem o histórico");
                call_gemini_agentic(app, &auth, system_prompt, prompt, &[], image, sa).await?
            }
            other => other?,
        };
        return Ok(JarvisReply {
            text,
            provider_used: provider.to_string(),
            actions,
            pending_actions,
        });
    }

    // Providers without native function calling act through [[ACTION:…]] tags.
    let tagged = format!("{system_prompt}\n{}", JARVIS_SYSTEM_PROMPT_TAGS.trim());
    let system_prompt = tagged.as_str();
    let (raw, provider) = match route {
        Route::Groq(k) => (call_groq(k, system_prompt, prompt, history).await?, "Groq"),
        Route::OpenAi(k) => (
            call_openai(k, system_prompt, prompt, history, image).await?,
            if image.is_some() { "OpenAI Vision" } else { "OpenAI (ChatGPT)" },
        ),
        Route::Claude(t) => (call_claude_oauth(t, system_prompt, prompt, history).await?, "Claude (Conta)"),
        Route::ClaudeCli => (call_claude_cli(system_prompt, prompt, history).await?, "Claude CLI (Conta)"),
        Route::GeminiCli => (call_gemini_cli(system_prompt, prompt, history).await?, "Gemini CLI (Conta)"),
        Route::Ollama(url) => (call_ollama(url, system_prompt, prompt, history).await?, "Ollama (Local)"),
        Route::GeminiKey(_) | Route::GeminiOAuth(_) => unreachable!("handled above"),
    };
    let (text, actions, pending_actions) = extract_and_run_actions(app, &raw, sa);
    Ok(JarvisReply {
        text,
        provider_used: provider.to_string(),
        actions,
        pending_actions,
    })
}


/// Transcription keys that just hit a quota (key tail → until when).
static STT_COOLDOWN: LazyLock<Mutex<HashMap<String, std::time::Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn stt_key(provider: &str, token: &str) -> String {
    format!("{provider}:{}", token.chars().rev().take(6).collect::<String>())
}

fn stt_resting(key: &str) -> bool {
    STT_COOLDOWN.lock().ok().and_then(|m| m.get(key).copied()).is_some_and(|t| std::time::Instant::now() < t)
}

fn stt_rest(key: &str, secs: u64) {
    if let Ok(mut m) = STT_COOLDOWN.lock() {
        m.insert(key.to_string(), std::time::Instant::now() + std::time::Duration::from_secs(secs));
    }
}

fn is_quota(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("429") || e.contains("quota") || e.contains("rate limit") || e.contains("rate_limit") || e.contains("resource_exhausted")
}

/// One Groq Whisper call. Err carries the reason (and "429" on quota).
async fn groq_transcribe(key: &str, bytes: Vec<u8>, mime: &str) -> Result<String, String> {
    let ext = if mime.contains("wav") {
        "wav"
    } else if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("mp4") || mime.contains("m4a") {
        "m4a"
    } else {
        "webm"
    };
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(format!("voice.{ext}"))
        .mime_str(mime)
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", GROQ_WHISPER_MODEL)
        .text("language", "pt")
        .text("prompt", whisper_prompt())
        .text("temperature", "0")
        .text("response_format", "json");
    let res = http_client()?
        .post(GROQ_WHISPER_URL)
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| net_err("Groq Whisper", e))?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.map_err(|e| format!("Resposta inválida do Whisper: {e}"))?;
    if status.is_success() {
        return Ok(drop_prompt_echo(body["text"].as_str().unwrap_or("")));
    }
    Err(format!("Groq Whisper ({status}): {}", body["error"]["message"].as_str().unwrap_or("falha na transcrição")))
}

/// Speech to text. Tries every saved Groq key, then every Gemini key, then the
/// Gemini account; a key that hits its quota rests for a while (60 s on Groq,
/// 5 min on Gemini) so the others take over instead of failing every time.
#[tauri::command]
pub async fn jarvis_transcribe(audio_base64: String, mime_type: String) -> Result<String, String> {
    let trimmed_b64 = audio_base64.trim();
    let bytes = decode_base64(trimmed_b64).map_err(|e| format!("Áudio base64 inválido: {e}"))?;
    if bytes.is_empty() {
        return Err("Áudio vazio".into());
    }
    let mime = if mime_type.trim().is_empty() {
        "audio/webm".to_string()
    } else {
        mime_type.split(';').next().unwrap_or("audio/webm").trim().to_string()
    };

    let mut errors: Vec<String> = Vec::new();
    let mut tried = 0;

    // Groq: every key, the active one first (legacy single key as a fallback).
    let mut groq: Vec<String> = jarvis_accounts::usable_accounts("groq").into_iter().map(|(_, t)| t).collect();
    if let Some((t, _)) = jarvis_accounts::get_active_credentials("groq") {
        if !groq.contains(&t) {
            groq.insert(0, t);
        }
    }
    for key in &groq {
        let id = stt_key("groq", key);
        if stt_resting(&id) {
            continue;
        }
        tried += 1;
        match groq_transcribe(key, bytes.clone(), &mime).await {
            Ok(text) => return Ok(text),
            Err(e) => {
                if is_quota(&e) {
                    stt_rest(&id, 60);
                }
                errors.push(e);
            }
        }
    }

    // Gemini: every API key, then the signed-in account.
    let mut gemini: Vec<GeminiAuth> = jarvis_accounts::usable_accounts("gemini").into_iter().map(|(_, t)| GeminiAuth::ApiKey(t)).collect();
    match jarvis_accounts::get_active_credentials("gemini") {
        Some((t, m)) if m == "oauth" => gemini.push(GeminiAuth::OAuth(t)),
        Some((t, _)) if !gemini.iter().any(|g| matches!(g, GeminiAuth::ApiKey(k) if *k == t)) => gemini.insert(0, GeminiAuth::ApiKey(t)),
        _ => {}
    }
    if let Some(t) = read_gemini_oauth_token() {
        if !gemini.iter().any(|g| matches!(g, GeminiAuth::OAuth(k) if *k == t)) {
            gemini.push(GeminiAuth::OAuth(t));
        }
    }
    for auth in &gemini {
        let token = match auth {
            GeminiAuth::ApiKey(t) | GeminiAuth::OAuth(t) => t.clone(),
        };
        let id = stt_key("gemini", &token);
        if stt_resting(&id) {
            continue;
        }
        tried += 1;
        match call_gemini_transcribe(auth, trimmed_b64, &mime).await {
            Ok(text) => return Ok(text),
            Err(e) => {
                if is_quota(&e) {
                    stt_rest(&id, 300);
                }
                errors.push(e);
            }
        }
    }

    if tried == 0 && (!groq.is_empty() || !gemini.is_empty()) {
        return Err("429: todas as chaves de transcrição estão no limite agora; tente de novo em um minuto.".into());
    }
    match errors.len() {
        0 => Err("Conecte uma conta Groq ou Gemini em Ajustes → IA & Voz para transcrição de voz.".into()),
        1 => Err(errors.remove(0)),
        _ => Err(format!("Nenhuma chave conseguiu transcrever. {}", errors.join(" · "))),
    }
}

/// Synthesizes natural pt-BR neural speech via Edge TTS (`pt-BR-AntonioNeural`)
/// and returns base64-encoded MP3 bytes.
#[tauri::command]
pub async fn jarvis_tts(text: String) -> Result<String, String> {
    let clean = text.trim().to_string();
    if clean.is_empty() {
        return Err("Texto vazio".into());
    }
    let s = crate::settings::load();
    if s.jarvis_tts_voice == "windows" {
        // The offline Windows voice: the island speaks it itself.
        return Err("windows-voice".into());
    }
    let voice = if s.jarvis_tts_voice.is_empty() { crate::edge_tts::DEFAULT_VOICE.to_string() } else { s.jarvis_tts_voice.clone() };
    match crate::edge_tts::synthesize(&clean, &voice, s.jarvis_tts_rate).await {
        Ok(mp3) => return Ok(encode_base64(&mp3)),
        Err(e) => crate::log::line(format!("voz: {e}; tentando o edge-tts do Python")),
    }
    python_edge_tts(clean, voice, s.jarvis_tts_rate).await
}

/// The old path, kept as a fallback: `python -m edge_tts` (when installed).
async fn python_edge_tts(clean: String, voice: String, rate_pct: i32) -> Result<String, String> {
    let tmp = std::env::temp_dir().join(format!(
        "jarvis_tts_{}_{}.mp3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let tmp_str = tmp.to_string_lossy().to_string();
    let rate = format!("{}{}%", if rate_pct >= 0 { "+" } else { "" }, rate_pct);

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let mut cmd = std::process::Command::new("python");
        cmd.args(["-m", "edge_tts", "--voice", &voice, "--rate", &rate, "--text", &clean, "--write-media", &tmp_str]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let status = cmd.status().map_err(|e| format!("edge-tts: {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err("edge-tts falhou".into());
        }
        let bytes = std::fs::read(&tmp).map_err(|e| format!("read tts: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
        Ok(bytes)
    })
    .await
    .map_err(|e| format!("join tts: {e}"))??;

    Ok(encode_base64(&result))
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Erro HTTP client: {e}"))
}

fn net_err(provider: &str, e: reqwest::Error) -> String {
    use std::error::Error;
    if e.is_timeout() {
        return format!("Tempo esgotado ao conectar em {provider} (verifique sua conexão).");
    }
    if let Some(src) = e.source() {
        return format!("Erro de conexão em {provider}: {src}");
    }
    format!("Erro de conexão em {provider}")
}

async fn call_groq(
    api_key: &str,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
) -> Result<String, String> {
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    for m in history.iter().rev().take(10).rev() {
        messages.push(json!({
            "role": if m.role == "assistant" { "assistant" } else { "user" },
            "content": m.content,
        }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));

    let client = http_client()?;
    let mut model = groq_model(api_key, false).await;
    let mut tried_refresh = false;
    let (status, body) = loop {
        let res = client
            .post(GROQ_CHAT_URL)
            .bearer_auth(api_key)
            .json(&json!({
                "model": model,
                "messages": messages,
                "temperature": 0.6,
                "max_tokens": 2048,
            }))
            .send()
            .await
            .map_err(|e| net_err("Groq", e))?;
        let status = res.status();
        let body: serde_json::Value = res.json().await.map_err(|e| format!("Resposta inválida do Groq: {e}"))?;
        // The model was retired: read the account's list and try once more.
        let gone = status.as_u16() == 404 || body["error"]["code"].as_str() == Some("model_not_found") || body["error"]["code"].as_str() == Some("model_decommissioned");
        if gone && !tried_refresh {
            tried_refresh = true;
            model = groq_model(api_key, true).await;
            continue;
        }
        break (status, body);
    };

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Erro na API do Groq");
        return Err(format!("Groq ({status}): {msg}"));
    }

    body["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Groq retornou resposta vazia".to_string())
}

async fn call_openai(
    api_key: &str,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
    image_base64: Option<&str>,
) -> Result<String, String> {
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    for m in history.iter().rev().take(10).rev() {
        messages.push(json!({
            "role": if m.role == "assistant" { "assistant" } else { "user" },
            "content": m.content,
        }));
    }
    if let Some(img) = image_base64 {
        messages.push(json!({
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{img}") } }
            ]
        }));
    } else {
        messages.push(json!({ "role": "user", "content": prompt }));
    }

    let client = http_client()?;
    let res = client
        .post(OPENAI_CHAT_URL)
        .bearer_auth(api_key)
        .json(&json!({
            "model": OPENAI_MODEL,
            "messages": messages,
            "temperature": 0.6,
            "max_tokens": 2048,
        }))
        .send()
        .await
        .map_err(|e| net_err("OpenAI", e))?;

    let status = res.status();
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Resposta inválida da OpenAI: {e}"))?;

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Erro na API da OpenAI");
        return Err(format!("OpenAI ({status}): {msg}"));
    }

    body["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "OpenAI retornou resposta vazia".to_string())
}

#[derive(Clone)]
enum GeminiAuth {
    ApiKey(String),
    OAuth(String),
}

async fn call_gemini_generate(
    auth: &GeminiAuth,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = http_client()?;
    let mut active_oauth = match auth {
        GeminiAuth::OAuth(t) => t.clone(),
        GeminiAuth::ApiKey(_) => String::new(),
    };
    let mut refreshed_once = false;
    let mut last_err = String::new();
    let label = match auth {
        GeminiAuth::ApiKey(_) => "Gemini",
        GeminiAuth::OAuth(_) => "Gemini (Conta)",
    };

    let mut idx = 0usize;
    while idx < GEMINI_MODELS.len() {
        let model = GEMINI_MODELS[idx];
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        );
        let req_timeout = if idx == 0 {
            Duration::from_secs(15)
        } else {
            Duration::from_secs(25)
        };

        let mut body_json = payload.clone();
        if THINKING_UNSUPPORTED.load(Ordering::Relaxed) {
            if let Some(cfg) = body_json.get_mut("generationConfig").and_then(|c| c.as_object_mut()) {
                cfg.remove("thinkingConfig");
            }
        }
        let mut req = client.post(url).timeout(req_timeout).json(&body_json);
        req = match auth {
            GeminiAuth::ApiKey(key) => req.header("x-goog-api-key", key),
            GeminiAuth::OAuth(_) => req.bearer_auth(&active_oauth),
        };

        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = net_err(label, e);
                if idx + 1 < GEMINI_MODELS.len() {
                    idx += 1;
                    continue;
                }
                return Err(last_err);
            }
        };

        let status = res.status();
        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| format!("Resposta inválida do Gemini: {e}"))?;

        if status.is_success() {
            return Ok(body);
        }

        if matches!(auth, GeminiAuth::OAuth(_)) && status.as_u16() == 401 && !refreshed_once {
            refreshed_once = true;
            if let Ok(fresh) = crate::jarvis_accounts::refresh_active_google_token().await {
                active_oauth = fresh;
                continue;
            }
        }

        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Erro na API do Gemini");
        if status.as_u16() == 400
            && msg.to_lowercase().contains("thinking")
            && body_json["generationConfig"].get("thinkingConfig").is_some()
        {
            // This model does not take thinkingLevel: drop it from now on and retry.
            THINKING_UNSUPPORTED.store(true, Ordering::Relaxed);
            crate::log::line(format!("gemini: thinkingConfig rejected ({msg}); retrying without it"));
            continue;
        }
        last_err = format!("{label} ({status}): {msg}");
        let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
        if !retryable || idx + 1 >= GEMINI_MODELS.len() {
            return Err(last_err);
        }
        idx += 1;
    }

    Err(if last_err.is_empty() {
        "Gemini não retornou resposta".to_string()
    } else {
        last_err
    })
}

async fn call_gemini_transcribe(
    auth: &GeminiAuth,
    audio_base64: &str,
    mime_type: &str,
) -> Result<String, String> {
    let payload = json!({
        "contents": [{
            "role": "user",
            "parts": [
                {
                    "text": format!("Transcreva fielmente a fala deste áudio em português do Brasil. Retorne apenas a transcrição exata, sem aspas nem comentários. Se houver apenas silêncio ou ruído sem fala inteligível, retorne vazio. Palavras que podem aparecer (escreva-as assim): {}.", stt_vocabulary())
                },
                {
                    "inline_data": {
                        "mime_type": mime_type,
                        "data": audio_base64,
                    }
                }
            ]
        }],
        "generationConfig": {
            "temperature": 0.0,
            "maxOutputTokens": 256,
        }
    });
    let body = call_gemini_generate(auth, &payload).await?;
    let text = body["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(text)
}

fn gemini_tool_declarations() -> serde_json::Value {
    let mut decl = json!([{
        "functionDeclarations": [
            {
                "name": "spotify_play",
                "description": "Busca e inicia a reprodução de uma música, álbum, artista ou playlist específica no app Spotify do Windows (abrindo o URI e clicando no Play da página nova, sem retomar a música antiga).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "query": {
                            "type": "STRING",
                            "description": "Nome exato da música, álbum, artista ou playlist (ex: 'Dawn FM The Weeknd')."
                        },
                        "kind": {
                            "type": "STRING",
                            "enum": ["auto", "album", "track", "artist", "playlist"],
                            "description": "Tipo de item no Spotify."
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "media_control",
                "description": "Controla a reprodução de mídia atual (play para retomar, pause, play_pause, next, previous, mute).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "action": {
                            "type": "STRING",
                            "enum": ["play", "pause", "play_pause", "next", "previous", "mute"]
                        }
                    },
                    "required": ["action"]
                }
            },
            {
                "name": "set_volume",
                "description": "Ajusta instantaneamente o volume mestre do Windows para uma porcentagem exata (0 a 100) via CoreAudio.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "percent": {
                            "type": "INTEGER",
                            "description": "Volume de 0 a 100."
                        }
                    },
                    "required": ["percent"]
                }
            },
            {
                "name": "open_target",
                "description": "Abre um aplicativo, site ou busca no Windows (ex: 'spotify', 'chrome', 'vscode', 'terminal', 'explorer', 'calc', 'notepad', 'whatsapp', 'discord', 'settings', 'https://...', 'youtube:termo', 'google:termo', 'editor:D:\\Projetos\\nome' para abrir um projeto no editor de código do usuário (Antigravity IDE)).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "target": { "type": "STRING" }
                    },
                    "required": ["target"]
                }
            },
            {
                "name": "run_powershell",
                "description": "Executa diretamente um comando PowerShell no Windows como Administrador e retorna stdout/stderr (timeout 20s). Use para arquivos, sistema, configurações, consultas ou automações do PC.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "command": { "type": "STRING", "description": "Comando PowerShell completo." },
                        "description": { "type": "STRING", "description": "Resumo curto em português do que o comando faz." }
                    },
                    "required": ["command", "description"]
                }
            },
            {
                "name": "get_pc_state",
                "description": "Lê o estado ao vivo do PC (volume, Spotify atual, janelas visíveis e opcionalmente processos top 10 ou conteúdo da área de transferência).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "include_processes": { "type": "BOOLEAN" },
                        "include_clipboard": { "type": "BOOLEAN" }
                    }
                }
            },
            {
                "name": "jarvis_action",
                "description": "Executa ações internas do Jarvis: salvar memória, criar tarefa, lembrete, timer, rotina personalizada, rodar rotina ou abrir aba.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "kind": {
                            "type": "STRING",
                            "enum": ["memory", "task", "reminder", "timer", "routine", "run_routine", "tab"]
                        },
                        "value": { "type": "STRING", "description": "Título, fato, nome da rotina, minutos ou nome da aba." },
                        "extra": { "type": "STRING", "description": "Horário HH:MM, descrição da rotina, categoria de memória ou nome do timer." },
                        "extra2": { "type": "STRING", "description": "Alvos da rotina separados por vírgula ou notas da memória." }
                    },
                    "required": ["kind", "value"]
                }
            },
            {
                "name": "delegate_openbot",
                "description": "Delega tarefas de engenharia de software/projetos (código, build, API, design, Obsidian) para o OpenBot executar.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "command": { "type": "STRING", "description": "Descrição clara da tarefa para o OpenBot." },
                        "agent": {
                            "type": "STRING",
                            "enum": ["orquestrador", "frontend-mobile", "backend-dados", "devops", "ui-ux"]
                        }
                    },
                    "required": ["command"]
                }
            },
            {
                "name": "browser_open",
                "description": "Abre um site (ou busca no Google se não for URL) no navegador do Jarvis, que você controla. Retorna o snapshot da página: elementos numerados [n] que dá para clicar/digitar e o texto visível. Use SEMPRE este para sites (não open_target).",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "url": { "type": "STRING", "description": "URL ou termo de busca." },
                        "new_tab": { "type": "BOOLEAN", "description": "Abrir em nova aba." }
                    },
                    "required": ["url"]
                }
            },
            {
                "name": "browser_snapshot",
                "description": "Lê o estado atual da página do navegador do Jarvis: título, URL, elementos numerados [n] e texto visível.",
                "parameters": { "type": "OBJECT", "properties": {} }
            },
            {
                "name": "browser_click",
                "description": "Clica (clique real de mouse) em um elemento da página pelo número [n] do último snapshot, ou pelo texto visível. Retorna o novo snapshot para você conferir se funcionou.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "element": { "type": "INTEGER", "description": "Número [n] do snapshot." },
                        "text": { "type": "STRING", "description": "Texto visível do elemento, se não tiver o número." }
                    }
                }
            },
            {
                "name": "browser_type",
                "description": "Digita texto em um campo (pelo número [n] ou texto/placeholder), substituindo o conteúdo. submit=true aperta Enter depois. Retorna o valor final do campo e o novo snapshot.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "element": { "type": "INTEGER" },
                        "field": { "type": "STRING", "description": "Texto/placeholder do campo, se não tiver o número." },
                        "text": { "type": "STRING" },
                        "submit": { "type": "BOOLEAN" }
                    },
                    "required": ["text"]
                }
            },
            {
                "name": "browser_select",
                "description": "Escolhe uma opção em uma lista <select> (pelo número [n]). Para menus personalizados use browser_click no menu e depois na opção.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "element": { "type": "INTEGER" },
                        "field": { "type": "STRING" },
                        "option": { "type": "STRING", "description": "Texto da opção." }
                    },
                    "required": ["option"]
                }
            },
            {
                "name": "browser_press",
                "description": "Pressiona uma tecla na página: Enter, Tab, Escape, ArrowDown, ArrowUp, PageDown, PageUp, Backspace, Space.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": { "key": { "type": "STRING" } },
                    "required": ["key"]
                }
            },
            {
                "name": "browser_scroll",
                "description": "Rola a página (down, up, top, bottom) e retorna o novo snapshot.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": { "direction": { "type": "STRING", "enum": ["down", "up", "top", "bottom"] } },
                    "required": ["direction"]
                }
            },
            {
                "name": "browser_back",
                "description": "Volta para a página anterior.",
                "parameters": { "type": "OBJECT", "properties": {} }
            },
            {
                "name": "calendar_list",
                "description": "Lista os eventos do Google Agenda do usuário a partir de uma data, por N dias. Cada evento vem com [id: …] para alterar ou apagar.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "from": { "type": "STRING", "description": "Data inicial AAAA-MM-DD (padrão: hoje)." },
                        "days": { "type": "INTEGER", "description": "Quantos dias (padrão 1; semana = 7)." }
                    }
                }
            },
            {
                "name": "calendar_create",
                "description": "Cria um evento no Google Agenda do usuário.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "title": { "type": "STRING" },
                        "start": { "type": "STRING", "description": "Início local AAAA-MM-DDTHH:MM (ou AAAA-MM-DD se dia todo)." },
                        "end": { "type": "STRING", "description": "Fim local AAAA-MM-DDTHH:MM (opcional)." },
                        "duration_minutes": { "type": "INTEGER", "description": "Duração se não houver fim (padrão 60)." },
                        "all_day": { "type": "BOOLEAN" },
                        "location": { "type": "STRING" },
                        "description": { "type": "STRING" },
                        "remind_minutes": { "type": "INTEGER", "description": "Notificar quantos minutos antes (0 = na hora). Passe SEMPRE que o usuário pedir lembrete, notificação, aviso ou alarme; sem minutos ditos use 10 (ou 0 se for um lembrete de um horário exato)." }
                    },
                    "required": ["title", "start"]
                }
            },
            {
                "name": "calendar_update",
                "description": "Muda título, horário e/ou a notificação de um evento (pelo id do calendar_list). Novo início sem fim mantém a duração.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {
                        "event_id": { "type": "STRING" },
                        "title": { "type": "STRING" },
                        "start": { "type": "STRING", "description": "Novo início local AAAA-MM-DDTHH:MM." },
                        "end": { "type": "STRING" },
                        "remind_minutes": { "type": "INTEGER", "description": "Adiciona/troca a notificação: minutos antes (0 = na hora)." }
                    },
                    "required": ["event_id"]
                }
            },
            {
                "name": "calendar_delete",
                "description": "Apaga um evento (pelo id do calendar_list). Só quando o usuário pedir claramente.",
                "parameters": {
                    "type": "OBJECT",
                    "properties": { "event_id": { "type": "STRING" } },
                    "required": ["event_id"]
                }
            },
            {
                "name": "browser_read",
                "description": "Lê o texto completo da página atual (para resumir, responder perguntas ou achar informações).",
                "parameters": { "type": "OBJECT", "properties": {} }
            }
        ]
    }]);
    if let Some(list) = decl[0]["functionDeclarations"].as_array_mut() {
        list.extend(crate::jarvis_tools::declarations());
    }
    decl
}

fn describe_tool_status(name: &str, args: &serde_json::Value) -> (String, String) {
    if let Some(line) = crate::jarvis_tools::status_line(name, args) {
        return line;
    }
    match name {
        "spotify_play" => (
            "Abrindo e tocando no Spotify…".to_string(),
            args["query"].as_str().unwrap_or("").to_string(),
        ),
        "media_control" => (
            "Controlando reprodução…".to_string(),
            args["action"].as_str().unwrap_or("").to_string(),
        ),
        "set_volume" => (
            "Ajustando volume…".to_string(),
            format!("{}%", args["percent"].as_u64().unwrap_or(50)),
        ),
        "open_target" => (
            "Abrindo no Windows…".to_string(),
            args["target"].as_str().unwrap_or("").to_string(),
        ),
        "run_powershell" => (
            "Executando no sistema…".to_string(),
            args["description"]
                .as_str()
                .or_else(|| args["command"].as_str())
                .unwrap_or("")
                .to_string(),
        ),
        "get_pc_state" => (
            "Verificando estado do PC…".to_string(),
            "Janelas, áudio e sistema".to_string(),
        ),
        "jarvis_action" => (
            "Atualizando Jarvis…".to_string(),
            args["value"].as_str().unwrap_or("").to_string(),
        ),
        "delegate_openbot" => (
            "Enviando para o OpenBot…".to_string(),
            args["command"].as_str().unwrap_or("").to_string(),
        ),
        "browser_open" => ("Abrindo no navegador…".to_string(), args["url"].as_str().unwrap_or("").to_string()),
        "browser_snapshot" | "browser_read" => ("Lendo a página…".to_string(), String::new()),
        "browser_click" => (
            "Clicando na página…".to_string(),
            args["text"].as_str().map(str::to_string).unwrap_or_else(|| format!("elemento {}", args["element"])),
        ),
        "browser_type" => ("Digitando…".to_string(), args["text"].as_str().unwrap_or("").to_string()),
        "browser_select" => ("Selecionando…".to_string(), args["option"].as_str().unwrap_or("").to_string()),
        "browser_press" | "browser_scroll" | "browser_back" => ("Navegando…".to_string(), String::new()),
        "calendar_list" => ("Lendo sua agenda…".to_string(), String::new()),
        "calendar_create" => ("Criando evento…".to_string(), args["title"].as_str().unwrap_or("").to_string()),
        "calendar_update" => ("Atualizando evento…".to_string(), args["title"].as_str().unwrap_or("").to_string()),
        "calendar_delete" => ("Apagando evento…".to_string(), String::new()),
        _ => ("Executando ação…".to_string(), name.to_string()),
    }
}

/// Runs a calendar_* tool; listings carry event ids for later changes.
async fn run_calendar_tool(name: &str, args: &serde_json::Value) -> Result<String, String> {
    use crate::google_calendar as cal;
    if !cal::connected() {
        return Err("Google Agenda não conectado. Conecte na aba Agenda do Jarvis.".into());
    }
    let s = |k: &str| args[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    match name {
        "calendar_list" => {
            let from = match s("from") {
                Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| format!("Data inválida: {d}"))?,
                None => cal::today(),
            };
            let days = args["days"].as_u64().unwrap_or(1).clamp(1, 31) as u32;
            Ok(cal::describe(&cal::events(from, days).await?, true))
        }
        "calendar_create" => {
            let ev = cal::create(cal::NewEvent {
                title: s("title").unwrap_or(""),
                start: s("start").ok_or("Falta o início do evento.")?,
                end: s("end"),
                minutes: args["duration_minutes"].as_i64(),
                all_day: args["all_day"].as_bool().unwrap_or(false),
                location: s("location"),
                description: s("description"),
                remind: args["remind_minutes"].as_i64(),
            })
            .await?;
            let note = match args["remind_minutes"].as_i64() {
                Some(0) => " Notificação na hora.".to_string(),
                Some(m) => format!(" Notificação {m} min antes."),
                None => String::new(),
            };
            Ok(format!("Evento criado: {}.{note}\n{}", ev.title, cal::describe(std::slice::from_ref(&ev), true)))
        }
        "calendar_update" => cal::update(s("event_id").ok_or("Falta o id do evento.")?, s("title"), s("start"), s("end"), args["remind_minutes"].as_i64()).await,
        "calendar_delete" => {
            cal::delete(s("event_id").ok_or("Falta o id do evento.")?).await?;
            Ok("Evento apagado.".into())
        }
        other => Err(format!("Ferramenta desconhecida: {other}")),
    }
}

/// Runs a browser_* tool. The output always ends with a fresh page snapshot,
/// which is what the model must check before claiming success.
async fn run_browser_tool(name: &str, args: &serde_json::Value) -> Option<Result<String, String>> {
    use crate::jarvis_browser as b;
    let element = args["element"].as_u64().filter(|n| *n > 0);
    let field = args["field"].as_str().or(args["text"].as_str().filter(|_| name == "browser_click"));
    Some(match name {
        "browser_open" => b::open(args["url"].as_str().unwrap_or("about:blank"), args["new_tab"].as_bool().unwrap_or(false)).await,
        "browser_snapshot" => b::snapshot().await,
        "browser_click" => b::click(element, field).await,
        "browser_type" => {
            b::type_text(element, args["field"].as_str(), args["text"].as_str().unwrap_or(""), args["submit"].as_bool().unwrap_or(false), true).await
        }
        "browser_select" => b::select_option(element, args["field"].as_str(), args["option"].as_str().unwrap_or("")).await,
        "browser_press" => b::press(args["key"].as_str().unwrap_or("Enter")).await,
        "browser_scroll" => b::scroll(args["direction"].as_str().unwrap_or("down")).await,
        "browser_back" => b::back().await,
        "browser_read" => b::read_page().await,
        _ => return None,
    })
}

/// Media keys through the Spotify Web API when it is connected (exact and
/// verified); None means "use the system media keys instead".
pub(crate) async fn api_media_control(action: &str) -> Option<String> {
    if !crate::spotify_api::connected() {
        return None;
    }
    let (api_action, msg) = match action {
        "play" => ("play", "Reproduzindo no Spotify"),
        "pause" => ("pause", "Spotify pausado"),
        "next" => ("next", "Próxima música"),
        "previous" | "prev" => ("previous", "Música anterior"),
        "play_pause" | "toggle" => {
            let playing = crate::spotify_api::state().await.ok().flatten().map(|s| s.playing).unwrap_or(false);
            if playing { ("pause", "Spotify pausado") } else { ("play", "Reproduzindo no Spotify") }
        }
        _ => return None,
    };
    crate::spotify_api::control(api_action, None, None).await.ok().map(|_| msg.to_string())
}

async fn execute_native_tool(
    app: &AppHandle,
    name: &str,
    args: &serde_json::Value,
    selected_agent: Option<&str>,
    pending: &mut Vec<PendingAction>,
) -> (bool, String, String) {
    // Reading tools are harmless to repeat on another AI; everything else counts.
    if !matches!(
        name,
        "get_pc_state" | "browser_snapshot" | "browser_read" | "calendar_list" | "gmail_search" | "gmail_read" | "daily_briefing" | "file_search" | "notes_search" | "notes_read" | "internet_status" | "github_overview"
    ) {
        TOOL_RUNS.fetch_add(1, Ordering::Relaxed);
    }
    {
        let mut queue = |kind: &str, cmd: &str, desc: &str| {
            let a = queue_pending(kind, cmd, desc);
            pending.push(a.clone());
            a
        };
        if let Some(r) = crate::jarvis_tools::run(app, name, args, &mut queue).await {
            return r;
        }
    }
    if name.starts_with("calendar_") {
        return match run_calendar_tool(name, args).await {
            Ok(out) => {
                let first = out.lines().next().unwrap_or("").to_string();
                (true, out, first)
            }
            Err(err) => (false, err.clone(), format!("Erro na agenda: {err}")),
        };
    }
    if name.starts_with("browser_") {
        let (title, _) = describe_tool_status(name, args);
        return match run_browser_tool(name, args).await {
            Some(Ok(out)) => {
                let first = out.lines().next().unwrap_or("").to_string();
                (true, out, format!("{}: {first}", title.trim_end_matches('…')))
            }
            Some(Err(err)) => (false, err.clone(), format!("Erro no navegador: {err}")),
            None => (false, format!("Ferramenta desconhecida: {name}"), format!("Ferramenta desconhecida: {name}")),
        };
    }
    match name {
        "spotify_play" => {
            let query = args["query"].as_str().unwrap_or("").trim().to_string();
            let kind = args["kind"]
                .as_str()
                .map(str::trim)
                .filter(|k| !k.is_empty() && *k != "auto")
                .map(ToString::to_string);
            match crate::spotify::resolve_and_play_spotify(app.clone(), query.clone(), kind).await {
                Ok(msg) => (true, msg.clone(), msg),
                Err(err) => (false, err.clone(), format!("Erro Spotify: {err}")),
            }
        }
        "media_control" => {
            let act = args["action"].as_str().unwrap_or("play_pause").trim().to_string();
            let lower = act.to_lowercase();
            // Something other than Spotify is playing (YouTube, VLC…): that one.
            let other = tokio::task::spawn_blocking(crate::media_sessions::now).await.ok().flatten();
            if let Some(m) = other.filter(|m| m.playing && m.app != "Spotify") {
                if matches!(lower.as_str(), "play" | "pause" | "play_pause" | "toggle" | "next" | "previous" | "prev") {
                    let l2 = lower.clone();
                    if let Ok(Ok(msg)) = tokio::task::spawn_blocking(move || crate::media_sessions::control(&l2)).await {
                        return (true, msg.clone(), msg);
                    }
                }
                let _ = m;
            }
            if let Some(msg) = api_media_control(&lower).await {
                return (true, msg.clone(), msg);
            }
            let res: Result<String, String> = if matches!(lower.as_str(), "play" | "pause") {
                if crate::spotify::spotify_control(app.clone(), lower.clone(), None).await {
                    Ok(if lower == "play" { "Reproduzindo".into() } else { "Pausado".into() })
                } else {
                    jarvis_pc::jarvis_media_control(act)
                }
            } else {
                jarvis_pc::jarvis_media_control(act)
            };
            match res {
                Ok(msg) => (true, msg.clone(), msg),
                Err(err) => (false, err.clone(), format!("Erro mídia: {err}")),
            }
        }
        "set_volume" => {
            let pct = args["percent"].as_u64().unwrap_or(50).min(100) as u32;
            match jarvis_pc::jarvis_set_volume(pct) {
                Ok(msg) => (true, msg.clone(), msg),
                Err(err) => (false, err.clone(), format!("Erro volume: {err}")),
            }
        }
        "open_target" => {
            let target = args["target"].as_str().unwrap_or("").trim().to_string();
            if target.eq_ignore_ascii_case("jarvis_settings")
                || target.eq_ignore_ascii_case("settings")
                || target.eq_ignore_ascii_case("configuracoes")
                || target.eq_ignore_ascii_case("configurações")
            {
                let _ = app.emit_to(crate::island::WINDOW_LABEL, "jarvis-open-tab", "settings".to_string());
                let msg = "Abrindo Configurações do Jarvis".to_string();
                (true, msg.clone(), msg)
            } else {
                match jarvis_pc::jarvis_open_target(target) {
                    Ok(msg) => (true, msg.clone(), msg),
                    Err(err) => (false, err.clone(), format!("Erro ao abrir: {err}")),
                }
            }
        }
        "run_powershell" => {
            let cmd = args["command"].as_str().unwrap_or("").trim().to_string();
            let desc = args["description"].as_str().unwrap_or("PowerShell").trim().to_string();
            if jarvis_config::confirm_system_actions() {
                pending.push(queue_pending("powershell", &cmd, &desc));
                return (true, AWAITING_CONFIRMATION.to_string(), format!("Aguardando confirmação: {desc}"));
            }
            let res = tokio::task::spawn_blocking(move || jarvis_pc::execute_pc_action("powershell", &cmd))
                .await
                .unwrap_or_else(|e| Err(format!("Falha thread PowerShell: {e}")));
            match res {
                Ok(out) => {
                    let first = out.lines().next().unwrap_or("Executado").trim();
                    (true, out.clone(), format!("{desc}: {first}"))
                }
                Err(err) => (false, err.clone(), format!("Erro ({desc}): {err}")),
            }
        }
        "get_pc_state" => {
            let inc_proc = args["include_processes"].as_bool().unwrap_or(false);
            let inc_clip = args["include_clipboard"].as_bool().unwrap_or(false);
            let snap = tokio::task::spawn_blocking(move || jarvis_pc::get_live_pc_snapshot(inc_proc, inc_clip))
                .await
                .unwrap_or_default();
            (true, snap, "Estado do PC verificado".to_string())
        }
        "jarvis_action" => {
            let kind = args["kind"].as_str().unwrap_or("").trim();
            let val = args["value"].as_str().unwrap_or("").trim();
            let extra = args["extra"].as_str().unwrap_or("").trim();
            let extra2 = args["extra2"].as_str().unwrap_or("").trim();
            let res: Result<String, String> = match kind {
                "tab" => {
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, "jarvis-open-tab", val.to_string());
                    Ok(format!("Abrindo aba {val}"))
                }
                "memory" => {
                    let cat = if extra.is_empty() { "fact" } else { extra };
                    jarvis_store::add_memory_internal(val, cat, extra2, None);
                    Ok(format!("Memória salva: {val}"))
                }
                "task" => {
                    jarvis_store::add_task_internal(val);
                    Ok(format!("Tarefa criada: {val}"))
                }
                "reminder" => {
                    let time = if extra.is_empty() { "09:00" } else { extra };
                    jarvis_store::add_reminder_internal(val, time);
                    Ok(format!("Lembrete criado: {val} às {time}"))
                }
                "timer" => {
                    let mins: u32 = val.parse().unwrap_or(5).max(1);
                    let label = if extra.is_empty() { "Timer" } else { extra };
                    let _ = app.emit_to(
                        crate::island::WINDOW_LABEL,
                        "jarvis-start-timer",
                        json!({ "minutes": mins, "label": label }),
                    );
                    Ok(format!("Timer iniciado: {label} ({mins} min)"))
                }
                "routine" => {
                    let desc = if extra.is_empty() { "Comando personalizado" } else { extra };
                    let targets: Vec<String> = extra2
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    jarvis_store::add_routine_internal(val, desc, targets);
                    Ok(format!("Rotina '{val}' criada"))
                }
                "run_routine" => crate::routines::run_named(app, val).await,
                other => Err(format!("Ação Jarvis desconhecida: {other}")),
            };
            match res {
                Ok(msg) => (true, msg.clone(), msg),
                Err(err) => (false, err.clone(), format!("Erro: {err}")),
            }
        }
        "delegate_openbot" => {
            let cmd = args["command"].as_str().unwrap_or("").trim().to_string();
            let ag = normalize_agent(
                args["agent"]
                    .as_str()
                    .map(ToString::to_string)
                    .or_else(|| selected_agent.map(ToString::to_string)),
            );
            let label_ag = ag.clone().unwrap_or_else(|| "orquestrador".to_string());
            match send_to_openbot_webhook(cmd, None, ag).await {
                Ok(_) => {
                    let msg = format!("Comando enviado ao OpenBot ({label_ag})");
                    (true, msg.clone(), msg)
                }
                Err(err) => (false, err.clone(), format!("Erro OpenBot: {err}")),
            }
        }
        other => (
            false,
            format!("Ferramenta desconhecida: {other}"),
            format!("Ferramenta desconhecida: {other}"),
        ),
    }
}

/// Why a Gemini answer came back without text, in words the user can act on.
fn empty_reason(body: &serde_json::Value) -> String {
    if let Some(block) = body["promptFeedback"]["blockReason"].as_str() {
        return format!("pedido bloqueado pelo filtro do Google: {block}");
    }
    match body["candidates"][0]["finishReason"].as_str().unwrap_or("") {
        "MAX_TOKENS" => "o limite de tamanho da resposta acabou antes do texto".into(),
        "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" => "resposta bloqueada pelo filtro de segurança do Google".into(),
        "RECITATION" => "resposta bloqueada por conter texto protegido".into(),
        "MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL" => "o modelo montou uma chamada de ferramenta inválida".into(),
        "" if body["candidates"].as_array().map_or(true, |c| c.is_empty()) => "nenhuma resposta veio do servidor".into(),
        "" | "STOP" => "o modelo terminou sem escrever nada".into(),
        other => format!("motivo: {other}"),
    }
}

async fn call_gemini_agentic(
    app: &AppHandle,
    auth: &GeminiAuth,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
    image_base64: Option<&str>,
    selected_agent: Option<&str>,
) -> Result<(String, Vec<String>, Vec<PendingAction>), String> {
    let started = Instant::now();
    let mut pending: Vec<PendingAction> = Vec::new();
    let mut contents = Vec::new();
    for m in history.iter().rev().take(10).rev() {
        contents.push(json!({
            "role": if m.role == "assistant" { "model" } else { "user" },
            "parts": [{ "text": m.content }],
        }));
    }
    let mut user_parts = vec![json!({ "text": prompt })];
    if let Some(img) = image_base64 {
        user_parts.push(json!({
            "inline_data": {
                "mime_type": "image/png",
                "data": img,
            }
        }));
    }
    contents.push(json!({
        "role": "user",
        "parts": user_parts,
    }));

    let tools = gemini_tool_declarations();
    let mut executed_actions = Vec::new();

    // Browser work takes more steps than "toca X": it gets a longer budget.
    let mut budget = AGENT_BUDGET;
    // Set after an empty answer: the retry goes without the thinking config.
    let mut lean = false;
    let mut retried_empty = false;
    for _step in 0..12 {
        let mut generation = json!({
            "temperature": 0.4,
            // Thinking tokens count against this limit: keep plenty of room,
            // or the model can spend it all thinking and answer nothing.
            "maxOutputTokens": 8192,
        });
        if !lean {
            // Commands need decisions, not essays: minimal thinking is the
            // single biggest latency win. Stripped automatically if the
            // model does not accept it (see call_gemini_generate).
            generation["thinkingConfig"] = json!({ "thinkingLevel": "low" });
        }
        let payload = json!({
            "systemInstruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": contents,
            "tools": tools,
            "generationConfig": generation,
        });

        // Once something ran, an error must not bubble up: the caller would fall
        // back to another provider and run the same actions twice.
        let remaining = budget.saturating_sub(started.elapsed());
        let result = if remaining.is_zero() {
            Err("tempo limite do Jarvis atingido".to_string())
        } else {
            match tokio::time::timeout(remaining, call_gemini_generate(auth, &payload)).await {
                Ok(r) => r,
                Err(_) => Err(format!("tempo limite de {}s atingido", budget.as_secs())),
            }
        };
        let body = match result {
            Ok(b) => b,
            Err(e) if executed_actions.is_empty() => return Err(e),
            Err(e) => {
                return Ok((
                    format!(
                        "Ações executadas:\n- {}\n(Não consegui concluir a resposta: {e})",
                        executed_actions.join("\n- ")
                    ),
                    executed_actions,
                    pending,
                ))
            }
        };
        let candidate_content = body["candidates"][0]["content"].clone();
        let parts = candidate_content["parts"].as_array().cloned().unwrap_or_default();

        let function_calls: Vec<serde_json::Value> = parts
            .iter()
            .filter_map(|p| p.get("functionCall").cloned())
            .collect();

        if function_calls.is_empty() {
            let text_parts: Vec<&str> = parts
                .iter()
                // Thought summaries are not the answer.
                .filter(|p| !p["thought"].as_bool().unwrap_or(false))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            let raw_text = text_parts.join("\n").trim().to_string();
            if raw_text.is_empty() {
                if !executed_actions.is_empty() {
                    return Ok((executed_actions.join("\n"), executed_actions, pending));
                }
                let reason = empty_reason(&body);
                crate::log::line(format!(
                    "gemini empty answer: {reason} | usage={} | lean={lean}",
                    body["usageMetadata"]
                ));
                if !retried_empty {
                    // Most often the thinking ate the turn: ask again, plainly.
                    retried_empty = true;
                    lean = true;
                    emit_status(app, "thinking", "Jarvis está pensando…", "");
                    continue;
                }
                return Err(format!("Gemini não respondeu ({reason})."));
            }
            if raw_text.contains("[[ACTION:") {
                let (clean, extra_actions, extra_pending) =
                    extract_and_run_actions(app, &raw_text, selected_agent);
                executed_actions.extend(extra_actions);
                pending.extend(extra_pending);
                return Ok((clean, executed_actions, pending));
            }
            return Ok((raw_text, executed_actions, pending));
        }

        // Preserve exact model content (including any thoughtSignature) for multi-turn FC compliance
        contents.push(candidate_content);

        let mut response_parts = Vec::new();
        for fc in function_calls {
            let fn_name = fc["name"].as_str().unwrap_or("");
            let fn_args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
            if fn_name.starts_with("browser_") {
                budget = BROWSER_BUDGET;
            }
            let (status_title, status_detail) = describe_tool_status(fn_name, &fn_args);
            emit_status(app, "executing", &status_title, &status_detail);

            let (ok, output, log_entry) =
                execute_native_tool(app, fn_name, &fn_args, selected_agent, &mut pending).await;
            executed_actions.push(log_entry);

            response_parts.push(json!({
                "functionResponse": {
                    "name": fn_name,
                    "response": {
                        "ok": ok,
                        "output": output,
                    }
                }
            }));
        }

        contents.push(json!({
            "role": "user",
            "parts": response_parts,
        }));
        emit_status(app, "thinking", "Jarvis está analisando o resultado…", "");
    }

    if !executed_actions.is_empty() {
        Ok((
            format!("Ações executadas:\n- {}", executed_actions.join("\n- ")),
            executed_actions,
            pending,
        ))
    } else {
        Err("Limite de iterações agênticas atingido.".to_string())
    }
}

async fn call_ollama(
    base_url: &str,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
) -> Result<String, String> {
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    for m in history.iter().rev().take(10).rev() {
        messages.push(json!({
            "role": if m.role == "assistant" { "assistant" } else { "user" },
            "content": m.content,
        }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));

    let client = http_client()?;
    let res = client
        .post(url)
        .json(&json!({
            "model": "llama3.2",
            "messages": messages,
            "stream": false,
        }))
        .send()
        .await
        .map_err(|e| format!("Erro ao conectar no Ollama: {e}"))?;

    let status = res.status();
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Resposta inválida do Ollama: {e}"))?;

    if !status.is_success() {
        return Err(format!("Ollama retornou status {status}"));
    }

    body["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Ollama retornou resposta vazia".to_string())
}

fn format_cli_prompt(system_prompt: &str, prompt: &str, history: &[BrainMessage]) -> String {
    let mut full = format!("Instruções de sistema:\n{system_prompt}\n");
    let recent: Vec<&BrainMessage> = history.iter().rev().take(6).collect();
    if !recent.is_empty() {
        full.push_str("\nHistórico recente:\n");
        for m in recent.into_iter().rev() {
            let role = if m.role == "assistant" { "Jarvis" } else { "Usuário" };
            full.push_str(&format!("{role}: {}\n", m.content));
        }
    }
    full.push_str(&format!("\nUsuário: {prompt}\nJarvis:"));
    full
}

async fn call_claude_cli(
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
) -> Result<String, String> {
    let bin = find_cli_binary("claude").ok_or_else(|| {
        "Claude CLI não encontrado. Instale o Claude Code ou use outra opção em Configurações → Cérebro."
            .to_string()
    })?;
    let full_prompt = format_cli_prompt(system_prompt, prompt, history);

    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut cmd = std::process::Command::new(&bin);
        cmd.args(["-p", &full_prompt, "--output-format", "text"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().map_err(|e| format!("Claude CLI: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(stdout)
        } else {
            let msg = if !stderr.is_empty() { stderr } else { stdout };
            if msg.to_lowercase().contains("login") || msg.to_lowercase().contains("auth") {
                Err("Claude CLI não está logado. Clique em 'Conectar Claude' ou execute `claude auth login`.".into())
            } else {
                Err(format!("Claude CLI falhou: {msg}"))
            }
        }
    })
    .await
    .map_err(|e| format!("join claude-cli: {e}"))?
}

async fn call_gemini_cli(
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
) -> Result<String, String> {
    let bin = find_cli_binary("gemini").ok_or_else(|| {
        "Gemini CLI não encontrado. Instale o Gemini CLI ou use outra opção em Configurações → Cérebro."
            .to_string()
    })?;
    let full_prompt = format_cli_prompt(system_prompt, prompt, history);

    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut cmd = std::process::Command::new(&bin);
        cmd.args(["-p", &full_prompt]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().map_err(|e| format!("Gemini CLI: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(stdout)
        } else {
            let msg = if !stderr.is_empty() { stderr } else { stdout };
            Err(format!("Gemini CLI falhou: {msg}"))
        }
    })
    .await
    .map_err(|e| format!("join gemini-cli: {e}"))?
}

async fn call_claude_oauth(
    token: &str,
    system_prompt: &str,
    prompt: &str,
    history: &[BrainMessage],
) -> Result<String, String> {
    let mut messages = Vec::new();
    for m in history.iter().rev().take(10).rev() {
        messages.push(json!({
            "role": if m.role == "assistant" { "assistant" } else { "user" },
            "content": m.content,
        }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));

    let client = http_client()?;
    let mut req = client
        .post(ANTHROPIC_MESSAGES_URL)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");

    if token.starts_with("sk-ant-api") {
        req = req.header("x-api-key", token);
    } else {
        req = req
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20");
    }

    let res = req
        .json(&json!({
            "model": CLAUDE_MODEL,
            "max_tokens": 2048,
            "system": system_prompt,
            "messages": messages,
        }))
        .send()
        .await
        .map_err(|e| net_err("Claude (Conta)", e))?;

    let status = res.status();
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Resposta inválida do Claude: {e}"))?;

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Erro na API do Claude");
        return Err(format!("Claude ({status}): {msg}"));
    }

    body["content"][0]["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Claude retornou resposta vazia".to_string())
}

fn extract_and_run_actions(
    app: &AppHandle,
    raw: &str,
    selected_agent: Option<&str>,
) -> (String, Vec<String>, Vec<PendingAction>) {
    let mut actions = Vec::new();
    let mut pending_actions = Vec::new();
    let confirm = jarvis_config::confirm_system_actions();
    let mut clean = String::with_capacity(raw.len());
    let mut rest = raw;
    let mut opened_spotify = false;

    while let Some(start) = rest.find("[[ACTION:") {
        clean.push_str(&rest[..start]);
        let after_prefix = &rest[start + 9..];
        if let Some(end) = after_prefix.find("]]") {
            let spec = &after_prefix[..end];
            if let Some((kind, arg)) = spec.split_once('|') {
                let a = arg.trim();
                let outcome = match kind.trim() {
                    "tab" => {
                        let _ = app.emit_to("island", "jarvis-open-tab", a.to_string());
                        Ok(format!("Abrindo aba {a}"))
                    }
                    "open" if a.eq_ignore_ascii_case("jarvis_settings") || a.eq_ignore_ascii_case("settings") || a.eq_ignore_ascii_case("configuracoes") || a.eq_ignore_ascii_case("configurações") => {
                        let _ = app.emit_to("island", "jarvis-open-tab", "settings".to_string());
                        Ok("Abrindo Configurações do Jarvis".to_string())
                    }
                    "open" => {
                        if a.to_lowercase().contains("spotify") {
                            opened_spotify = true;
                        }
                        jarvis_pc::jarvis_open_target(a.to_string())
                    }
                    "spotify_play" => {
                        opened_spotify = true;
                        let app_clone = app.clone();
                        let query = a.to_string();
                        tauri::async_runtime::spawn(async move {
                            let _ = crate::spotify::resolve_and_play_spotify(app_clone, query, None).await;
                        });
                        Ok(format!("Tocando no Spotify: {a}"))
                    }
                    "volume" => {
                        let digits: String = a.chars().filter(|c| c.is_ascii_digit()).collect();
                        match digits.parse::<u32>() {
                            Ok(pct) => jarvis_pc::jarvis_set_volume(pct),
                            Err(_) => Err(format!("Volume inválido: {a}")),
                        }
                    }
                    "media" => {
                        let lower_media = a.to_lowercase();
                        if opened_spotify && matches!(lower_media.as_str(), "play" | "play_pause" | "toggle") {
                            let app_clone = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = crate::spotify::spotify_play_when_ready(app_clone).await;
                            });
                            Ok("Aguardando Spotify iniciar para reproduzir".to_string())
                        } else if matches!(lower_media.as_str(), "play" | "pause") {
                            let app_clone = app.clone();
                            let act = lower_media.clone();
                            tauri::async_runtime::spawn(async move {
                                if !crate::spotify::spotify_control(app_clone, act.clone(), None).await {
                                    let _ = jarvis_pc::jarvis_media_control(act);
                                }
                            });
                            Ok(if lower_media == "play" { "Reproduzir" } else { "Pausar" }.to_string())
                        } else {
                            jarvis_pc::jarvis_media_control(a.to_string())
                        }
                    }
                    "memory" => {
                        let parts: Vec<&str> = a.split('|').map(|s| s.trim()).collect();
                        if parts.len() >= 3 {
                            let cat = parts[0];
                            let title = parts[1];
                            let notes = parts[2];
                            let conn = parts.get(3).copied();
                            jarvis_store::add_memory_internal(title, cat, notes, conn);
                            Ok(format!("Memória salva na malha: {title}"))
                        } else {
                            jarvis_store::add_memory_internal(a, "fact", "", None);
                            Ok(format!("Memória salva: {a}"))
                        }
                    }
                    "task" => {
                        jarvis_store::add_task_internal(a);
                        Ok(format!("Tarefa criada: {a}"))
                    }
                    "reminder" => {
                        let (title, time) = a.split_once('|').unwrap_or((a, "09:00"));
                        jarvis_store::add_reminder_internal(title.trim(), time.trim());
                        Ok(format!("Lembrete criado: {} às {}", title.trim(), time.trim()))
                    }
                    "timer" => {
                        let (mins_str, label) = a.split_once('|').unwrap_or((a, "Timer"));
                        let mins: u32 = mins_str.trim().parse().unwrap_or(5).max(1);
                        let _ = app.emit_to(
                            "island",
                            "jarvis-start-timer",
                            json!({ "minutes": mins, "label": label.trim() }),
                        );
                        Ok(format!("Timer iniciado: {} ({mins} min)", label.trim()))
                    }
                    "routine" => {
                        let parts: Vec<&str> = a.split('|').map(|s| s.trim()).collect();
                        if parts.len() >= 3 {
                            let targets: Vec<String> = parts[2]
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            jarvis_store::add_routine_internal(parts[0], parts[1], targets);
                            Ok(format!("Comando/Rotina '{}' criado!", parts[0]))
                        } else if parts.len() == 2 {
                            let targets: Vec<String> = parts[1]
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            jarvis_store::add_routine_internal(parts[0], "Comando personalizado", targets);
                            Ok(format!("Comando/Rotina '{}' criado!", parts[0]))
                        } else {
                            Err("Formato de rotina inválido".into())
                        }
                    }
                    "run_routine" => {
                        let data = jarvis_store::load_data();
                        if let Some(rt) = data.routines.iter().find(|r| r.name.eq_ignore_ascii_case(a)) {
                            for t in &rt.targets {
                                let _ = jarvis_pc::jarvis_open_target(t.clone());
                            }
                            Ok(format!("Executando rotina: {}", rt.name))
                        } else {
                            Err(format!("Rotina não encontrada: {a}"))
                        }
                    }
                    "powershell" if confirm => {
                        pending_actions.push(queue_pending("powershell", a, "PowerShell"));
                        Ok("Aguardando confirmação: PowerShell".to_string())
                    }
                    "powershell" => {
                        match jarvis_pc::execute_pc_action("powershell", a) {
                            Ok(out) => {
                                let first_line = out.lines().next().unwrap_or("Executado").trim();
                                Ok(format!("PowerShell: {first_line}"))
                            }
                            Err(e) => Err(e),
                        }
                    }
                    "pc_exec" => {
                        let (exec_kind, exec_cmd) = if let Some((first, second)) = a.split_once('|') {
                            let fk = first.trim().to_lowercase();
                            if matches!(
                                fk.as_str(),
                                "powershell" | "ps" | "terminal" | "cmd" | "open" | "app" | "url" | "process" | "file"
                            ) {
                                (first.trim(), second.trim())
                            } else {
                                ("powershell", a)
                            }
                        } else {
                            ("powershell", a)
                        };
                        let is_shell = matches!(
                            exec_kind.to_lowercase().as_str(),
                            "powershell" | "ps" | "terminal" | "cmd"
                        );
                        if confirm && is_shell {
                            pending_actions.push(queue_pending(exec_kind, exec_cmd, exec_kind));
                            actions.push(format!("Aguardando confirmação: {exec_kind}"));
                            rest = &after_prefix[end + 2..];
                            continue;
                        }
                        match jarvis_pc::execute_pc_action(exec_kind, exec_cmd) {
                            Ok(out) => {
                                let first_line = out.lines().next().unwrap_or("Executado").trim();
                                Ok(format!("{exec_kind}: {first_line}"))
                            }
                            Err(e) => Err(e),
                        }
                    }
                    "openbot" => {
                        let (cmd, ag) = if let Some((maybe_agent, rest_cmd)) = a.split_once('|') {
                            if is_known_agent(maybe_agent) {
                                (rest_cmd.trim().to_string(), normalize_agent(Some(maybe_agent.to_string())))
                            } else {
                                (a.to_string(), selected_agent.map(ToString::to_string))
                            }
                        } else {
                            (a.to_string(), selected_agent.map(ToString::to_string))
                        };
                        let label_ag = ag.clone().unwrap_or_else(|| "orquestrador".to_string());
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = send_to_openbot_webhook(cmd, None, ag).await {
                                if let Ok(appdata) = std::env::var("APPDATA") {
                                    let log_path = std::path::PathBuf::from(appdata)
                                        .join("OpenJarvis")
                                        .join("openbot_webhook.log");
                                    let _ = std::fs::write(&log_path, format!("ERR: {e}\n"));
                                }
                            }
                        });
                        Ok(format!("Comando enviado ao OpenBot ({label_ag})"))
                    }
                    other => Err(format!("Ação desconhecida: {other}")),
                };
                match outcome {
                    Ok(msg) => actions.push(msg),
                    Err(err) => actions.push(format!("Erro: {err}")),
                }
            }
            rest = &after_prefix[end + 2..];
        } else {
            clean.push_str("[[ACTION:");
            rest = after_prefix;
        }
    }
    clean.push_str(rest);
    (clean.trim().to_string(), actions, pending_actions)
}

/// Base64 to bytes, for other modules (WhatsApp images).
pub fn decode_base64_pub(input: &str) -> Result<Vec<u8>, String> {
    decode_base64(input.trim()).map_err(|e| e.to_string())
}

fn decode_base64(input: &str) -> Result<Vec<u8>, &'static str> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if cleaned.len() % 4 != 0 {
        return Err("comprimento inválido");
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    for chunk in cleaned.chunks_exact(4) {
        let mut vals = [0u8; 4];
        let mut pad = 0;
        for i in 0..4 {
            let b = chunk[i];
            if b == b'=' {
                pad += 1;
                vals[i] = 0;
            } else {
                vals[i] = match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a' + 26,
                    b'0'..=b'9' => b - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => return Err("caractere inválido"),
                };
            }
        }
        out.push((vals[0] << 2) | (vals[1] >> 4));
        if pad < 2 {
            out.push((vals[1] << 4) | (vals[2] >> 2));
        }
        if pad < 1 {
            out.push((vals[2] << 6) | vals[3]);
        }
    }
    Ok(out)
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        out.push(TABLE[b0 >> 2] as char);
        out.push(TABLE[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(if chunk.len() > 1 { TABLE[((b1 & 0x0f) << 2) | (b2 >> 6)] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[b2 & 0x3f] as char } else { '=' });
    }
    out
}

static DELIVERY_SEQ: AtomicU64 = AtomicU64::new(0);

/// A delivery id that does not repeat even for several sends in the same second.
fn openbot_delivery_id(ts: u64) -> String {
    let seq = DELIVERY_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut rnd = [0u8; 6];
    let _ = getrandom::getrandom(&mut rnd);
    format!("jarvis-{ts}-{}-{seq}-{}", std::process::id(), hex::encode(rnd))
}

pub async fn send_to_openbot_webhook(
    command: String,
    project: Option<String>,
    agent: Option<String>,
) -> Result<String, String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // The registry fallback runs PowerShell with CREATE_NO_WINDOW (no console flash).
    let secret = secrets::get("openbot-webhook-secret")
        .or_else(|| jarvis_accounts::read_env_or_hkcu("OPENBOT_WEBHOOK_SECRET"))
        .ok_or_else(|| {
            "Segredo do webhook OpenBot não configurado (openbot-webhook-secret).".to_string()
        })?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let delivery_id = openbot_delivery_id(ts);
    let webhook_url = jarvis_config::openbot_webhook_url();
    if !webhook_url.starts_with("https://") {
        return Err("O webhook do OpenBot precisa ser uma URL https://".to_string());
    }

    let payload = serde_json::json!({
        "type": "jarvis.command",
        "data": {
            "command": command,
            "project": project,
            "agent": agent,
        }
    });
    let body_bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(format!("{ts}.{delivery_id}.").as_bytes());
    mac.update(&body_bytes);
    let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let client = http_client()?;
    let res = client
        .post(&webhook_url)
        .header("content-type", "application/json")
        .header("x-openbot-timestamp", ts.to_string())
        .header("x-openbot-delivery-id", &delivery_id)
        .header("x-openbot-signature", sig)
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| net_err("OpenBot Webhook", e))?;

    let status = res.status();
    let resp_text = res.text().await.unwrap_or_default();
    if let Ok(appdata) = std::env::var("APPDATA") {
        let log_path = std::path::PathBuf::from(appdata).join("OpenJarvis").join("openbot_webhook.log");
        let _ = std::fs::write(&log_path, format!("ts={ts} status={status} body={resp_text}\n"));
    }

    if status.is_success() {
        Ok("Comando enviado ao OpenBot".to_string())
    } else {
        Err(format!("OpenBot Webhook ({status})"))
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn failover_tries_other_keys_then_other_ais() {
        let g1 = Route::GeminiKey("key-one-aaaaaa".into());
        let g2 = Route::GeminiKey("key-two-bbbbbb".into());
        let groq = Route::Groq("groq-cccccc".into());
        let plan = failover_plan(
            &[g1.clone()],
            &[g1.clone(), groq.clone()],
            &[(g1.clone(), Some("acc1".into())), (g2.clone(), Some("acc2".into()))],
        );
        let order: Vec<_> = plan.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(order, vec![g1.clone(), g2.clone(), groq.clone()]);
        assert_eq!(plan[1].1.as_deref(), Some("acc2"));

        // A key that just ran out of quota is tried last.
        cool_down(&g1, "429 RESOURCE_EXHAUSTED quota");
        let plan = failover_plan(&[g1.clone()], &[g1.clone(), groq.clone()], &[(g2.clone(), Some("acc2".into()))]);
        assert_eq!(plan.last().map(|(r, _)| r.clone()), Some(g1));
    }

    fn creds() -> Creds {
        Creds {
            groq: None,
            gemini_key: None,
            gemini_oauth: None,
            openai: None,
            claude: None,
            ollama_url: "http://localhost:11434".into(),
        }
    }

    #[test]
    fn auto_mode_lists_every_connected_provider_in_order() {
        let c = Creds {
            gemini_key: Some("g".into()),
            groq: Some("q".into()),
            openai: Some("o".into()),
            claude: Some("c".into()),
            ..creds()
        };
        let routes = plan_routes("auto", &c, false, true, false).unwrap();
        assert_eq!(
            routes,
            vec![
                Route::GeminiKey("g".into()),
                Route::Groq("q".into()),
                Route::OpenAi("o".into()),
                Route::Claude("c".into()),
                Route::ClaudeCli,
            ]
        );
    }

    #[test]
    fn auto_mode_without_any_account_says_what_to_do() {
        let err = plan_routes("auto", &creds(), false, false, false).unwrap_err();
        assert!(err.contains("Nenhuma conta conectada"));
    }

    #[test]
    fn a_specific_mode_never_falls_back_to_another_provider() {
        let c = Creds { groq: Some("q".into()), gemini_key: Some("g".into()), ..creds() };
        assert_eq!(plan_routes("groq", &c, false, false, false).unwrap(), vec![Route::Groq("q".into())]);
        assert!(plan_routes("openai", &c, false, false, false).is_err());
    }

    #[test]
    fn images_only_go_to_vision_providers() {
        let c = Creds { groq: Some("q".into()), openai: Some("o".into()), ..creds() };
        assert_eq!(plan_routes("auto", &c, true, true, true).unwrap(), vec![Route::OpenAi("o".into())]);
        let none = Creds { groq: Some("q".into()), ..creds() };
        assert!(plan_routes("auto", &none, true, false, false).is_err());
    }

    #[test]
    fn destructive_commands_are_flagged_as_danger() {
        assert_eq!(risk_level("Remove-Item C:\\temp -Recurse"), "danger");
        assert_eq!(risk_level("Stop-Process -Name chrome"), "danger");
        assert_eq!(risk_level("Get-Date"), "warning");
    }

    #[test]
    fn queued_actions_wait_until_confirmed_or_cancelled() {
        let a = queue_pending("powershell", "Get-Date", "Ver a hora");
        let b = queue_pending("powershell", "Get-Date", "Ver a hora");
        assert_ne!(a.id, b.id);
        assert!(jarvis_list_pending_actions().iter().any(|p| p.id == a.id));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let res = rt.block_on(jarvis_execute_confirmed_action(a.id.clone(), false)).unwrap();
        assert!(!res.executed);
        assert!(!jarvis_list_pending_actions().iter().any(|p| p.id == a.id));
        // A second answer for the same action is refused.
        assert!(rt.block_on(jarvis_execute_confirmed_action(a.id, true)).is_err());
        let _ = rt.block_on(jarvis_execute_confirmed_action(b.id, false));
    }

    #[test]
    fn agent_names_are_normalized() {
        assert_eq!(normalize_agent(Some("Frontend".into())).as_deref(), Some("frontend-mobile"));
        assert_eq!(normalize_agent(Some("UI/UX".into())).as_deref(), Some("ui-ux"));
        assert_eq!(normalize_agent(Some("  ".into())), None);
        assert!(is_known_agent("devops"));
        assert!(!is_known_agent("hacker"));
    }

    #[test]
    fn base64_round_trips() {
        for input in [&b""[..], b"a", b"ab", b"abc", b"Jarvis \xc3\xa9 legal"] {
            assert_eq!(decode_base64(&encode_base64(input)).unwrap(), input);
        }
        assert!(decode_base64("abc").is_err());
    }

    #[test]
    fn empty_gemini_answers_say_why() {
        let max = json!({ "candidates": [{ "finishReason": "MAX_TOKENS", "content": {} }] });
        assert!(empty_reason(&max).contains("limite"));
        let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert!(empty_reason(&blocked).contains("bloqueado"));
        assert!(empty_reason(&json!({ "candidates": [] })).contains("nenhuma resposta"));
        let stop = json!({ "candidates": [{ "finishReason": "STOP", "content": { "parts": [] } }] });
        assert!(empty_reason(&stop).contains("sem escrever"));
    }

    #[test]
    fn webhook_delivery_ids_never_repeat_in_the_same_second() {
        let a = openbot_delivery_id(1_700_000_000);
        let b = openbot_delivery_id(1_700_000_000);
        assert_ne!(a, b);
    }
}

#[cfg(test)]
mod voice_probe {
    /// Sends %TEMP%\jarvis-voice-test.wav through the real transcription path
    /// with the connected keys: `cargo test transcribes_a_sample -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn transcribes_a_sample() {
        let path = std::env::temp_dir().join("jarvis-voice-test.wav");
        let bytes = std::fs::read(&path).expect("gere o WAV de teste antes");
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut b64 = String::new();
        for c in bytes.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            b64.push(T[(n >> 18) as usize & 63] as char);
            b64.push(T[(n >> 12) as usize & 63] as char);
            b64.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            b64.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let started = std::time::Instant::now();
        let r = rt.block_on(super::jarvis_transcribe(b64, "audio/wav".into()));
        println!("{:?} em {:?}", r, started.elapsed());
    }
}
