//! The newer Jarvis tools, kept out of jarvis_brain.rs: Gmail (only when the
//! user asks), the "bom dia" briefing, files, Obsidian notes, audio outputs
//! and microphone, internet, GitHub and WhatsApp (WhatsApp Web in the Jarvis
//! browser, see whatsapp.rs; OpenBot as a fallback). Messages
//! that leave the PC (e-mail, WhatsApp) always wait for Permitir.

use serde_json::{json, Value};
use tauri::AppHandle;

/// Pending-action kinds that run here (async) instead of jarvis_pc.
pub const GMAIL_SEND: &str = "gmail_send";
pub const WHATSAPP: &str = "whatsapp";

pub fn declarations() -> Vec<Value> {
    vec![
        json!({
            "name": "gmail_search",
            "description": "Lista e-mails do Gmail do usuário. Use SOMENTE quando o usuário pedir explicitamente algo sobre e-mails (ex.: 'tenho e-mail novo?', 'resume meus e-mails', 'acha o e-mail da Ana'). Nunca chame por iniciativa própria.",
            "parameters": {"type": "OBJECT", "properties": {
                "query": {"type": "STRING", "description": "Busca do Gmail: 'is:unread', 'from:ana', 'newer_than:2d', 'subject:fatura'. Vazio = caixa de entrada."},
                "max": {"type": "INTEGER", "description": "Quantos (1-25), padrão 10."}
            }}
        }),
        json!({
            "name": "gmail_read",
            "description": "Lê o conteúdo completo de um e-mail (id vindo do gmail_search). Só quando o usuário pediu.",
            "parameters": {"type": "OBJECT", "properties": {"id": {"type": "STRING"}}, "required": ["id"]}
        }),
        json!({
            "name": "gmail_draft",
            "description": "Cria um RASCUNHO no Gmail (novo ou resposta). Nunca envia sozinho: o envio fica aguardando o usuário tocar em Permitir. Só quando o usuário pediu para escrever/responder um e-mail.",
            "parameters": {"type": "OBJECT", "properties": {
                "reply_to_id": {"type": "STRING", "description": "id do e-mail sendo respondido (opcional)."},
                "to": {"type": "STRING", "description": "Destinatário (obrigatório se não for resposta)."},
                "subject": {"type": "STRING"},
                "body": {"type": "STRING", "description": "Texto do e-mail, já escrito no tom do usuário."},
                "send": {"type": "BOOLEAN", "description": "true se o usuário pediu para ENVIAR (fica aguardando Permitir); false só salva o rascunho."}
            }, "required": ["body"]}
        }),
        json!({
            "name": "daily_briefing",
            "description": "Resumo do dia: clima, compromissos da agenda, tarefas e lembretes pendentes. Use para 'bom dia', 'como está meu dia', 'o que tenho hoje'.",
            "parameters": {"type": "OBJECT", "properties": {}}
        }),
        json!({
            "name": "file_search",
            "description": "Procura arquivos e pastas do usuário pelo nome (Área de Trabalho, Documentos, Downloads, Imagens, Vídeos, projetos). Ex.: 'pdf contrato março'.",
            "parameters": {"type": "OBJECT", "properties": {"query": {"type": "STRING"}}, "required": ["query"]}
        }),
        json!({
            "name": "file_open",
            "description": "Abre um arquivo (caminho completo vindo do file_search) ou mostra ele na pasta.",
            "parameters": {"type": "OBJECT", "properties": {
                "path": {"type": "STRING"},
                "reveal": {"type": "BOOLEAN", "description": "true = mostrar na pasta em vez de abrir."}
            }, "required": ["path"]}
        }),
        json!({
            "name": "app_read",
            "description": "Lê o que aparece na janela de um programa aberto no PC: textos, botões, campos e o que está neles (ex.: o resultado da calculadora, o texto do Bloco de Notas). Use depois de agir num programa para conferir.",
            "parameters": {"type": "OBJECT", "properties": {
                "app": {"type": "STRING", "description": "Nome do programa ou parte do título da janela ('calculadora', 'bloco de notas', 'excel'). Vazio = janela em foco."}
            }}
        }),
        json!({
            "name": "app_type",
            "description": "Digita texto e/ou aperta teclas numa janela de programa aberto, como uma pessoa no teclado. Ex.: calculadora: text '125*8='; salvar: keys 'ctrl+s'; keys aceita combinações separadas por espaço ('ctrl+a delete', 'alt+f4', 'enter', 'tab', 'f5').",
            "parameters": {"type": "OBJECT", "properties": {
                "app": {"type": "STRING", "description": "Nome do programa ou parte do título."},
                "text": {"type": "STRING", "description": "Texto a digitar (\\n = Enter)."},
                "keys": {"type": "STRING", "description": "Teclas/atalhos depois do texto."},
                "field": {"type": "STRING", "description": "Nome do campo onde digitar (opcional)."}
            }, "required": ["app"]}
        }),
        json!({
            "name": "app_click",
            "description": "Clica num botão, menu, aba ou item de um programa aberto pelo nome que aparece nele (use app_read para ver os nomes).",
            "parameters": {"type": "OBJECT", "properties": {
                "app": {"type": "STRING"},
                "target": {"type": "STRING", "description": "Nome do botão/item, ex.: 'Igual a', 'Arquivo', 'Salvar'."}
            }, "required": ["app", "target"]}
        }),
        json!({
            "name": "app_list",
            "description": "Lista as janelas de programas abertas no PC.",
            "parameters": {"type": "OBJECT", "properties": {}}
        }),
        json!({
            "name": "file_read",
            "description": "Lê o conteúdo de um arquivo de texto (txt, md, csv, json, código…) pelo caminho completo.",
            "parameters": {"type": "OBJECT", "properties": {"path": {"type": "STRING"}}, "required": ["path"]}
        }),
        json!({
            "name": "file_write",
            "description": "Cria ou edita um arquivo de texto. mode 'overwrite' (escreve tudo), 'append' (acrescenta no fim) ou 'replace' (troca o trecho find por replace). A versão anterior fica guardada numa cópia de segurança.",
            "parameters": {"type": "OBJECT", "properties": {
                "path": {"type": "STRING", "description": "Caminho completo."},
                "mode": {"type": "STRING", "enum": ["overwrite", "append", "replace"]},
                "content": {"type": "STRING"},
                "find": {"type": "STRING"},
                "replace": {"type": "STRING"}
            }, "required": ["path", "mode"]}
        }),
        json!({
            "name": "notes_search",
            "description": "Busca notas no Obsidian do usuário (título e texto). Vazio = notas mais recentes.",
            "parameters": {"type": "OBJECT", "properties": {"query": {"type": "STRING"}}}
        }),
        json!({
            "name": "notes_read",
            "description": "Lê uma nota do Obsidian (caminho do notes_search, ou o título).",
            "parameters": {"type": "OBJECT", "properties": {"path": {"type": "STRING"}}, "required": ["path"]}
        }),
        json!({
            "name": "notes_write",
            "description": "Anota algo no Obsidian: acrescenta à nota com esse título (cria se não existir). Ex.: 'anota no projeto jarvis que…' → title='jarvis'.",
            "parameters": {"type": "OBJECT", "properties": {
                "title": {"type": "STRING"},
                "text": {"type": "STRING"},
                "folder": {"type": "STRING", "description": "Pasta para nota nova (padrão 'Jarvis'; projetos em 'projetos')."}
            }, "required": ["title", "text"]}
        }),
        json!({
            "name": "audio_output",
            "description": "Troca a saída de som do Windows: 'fone', 'caixa', 'monitor', o nome do aparelho, ou vazio para a próxima. Também lista as saídas.",
            "parameters": {"type": "OBJECT", "properties": {"device": {"type": "STRING"}}}
        }),
        json!({
            "name": "mic_mute",
            "description": "Muta (true) ou liga (false) o microfone.",
            "parameters": {"type": "OBJECT", "properties": {"mute": {"type": "BOOLEAN"}}, "required": ["mute"]}
        }),
        json!({
            "name": "internet_status",
            "description": "Verifica a internet: ping, e opcionalmente mede a velocidade de download.",
            "parameters": {"type": "OBJECT", "properties": {"speed_test": {"type": "BOOLEAN"}}}
        }),
        json!({
            "name": "whatsapp_send",
            "description": "Manda uma mensagem de WhatsApp. A primeira chamada NÃO envia: ela prepara e você pergunta ao usuário se pode enviar. Só quando o usuário pediu para mandar mensagem.",
            "parameters": {"type": "OBJECT", "properties": {
                "to": {"type": "STRING", "description": "Nome do contato ou grupo como o usuário falou, um número com DDD, ou \"eu\" quando é para o próprio usuário (\"manda pra mim\")."},
                "message": {"type": "STRING"}
            }, "required": ["to", "message"]}
        }),
        json!({
            "name": "whatsapp_read",
            "description": "Lê as últimas mensagens de UMA conversa do WhatsApp. Só quando o usuário pediu para ler aquela conversa.",
            "parameters": {"type": "OBJECT", "properties": {
                "chat": {"type": "STRING", "description": "Nome do contato ou grupo, número, ou \"eu\" (conversa consigo mesmo)."},
                "count": {"type": "INTEGER", "description": "Quantas mensagens (padrão 10, máx. 40)."}
            }, "required": ["chat"]}
        }),
        json!({
            "name": "whatsapp_call",
            "description": "Faz uma ligação (ou chamada de vídeo) pelo WhatsApp. NÃO liga na hora: pergunte ao usuário se pode ligar; ao \"sim\" a ligação é feita. Só quando o usuário pedir.",
            "parameters": {"type": "OBJECT", "properties": {
                "to": {"type": "STRING", "description": "Nome do contato ou número."},
                "video": {"type": "BOOLEAN", "description": "true para chamada de vídeo."}
            }, "required": ["to"]}
        }),
        json!({
            "name": "obs_control",
            "description": "Controla o OBS Studio (precisa estar aberto): gravar, live, cenas, microfone, replay, câmera virtual e status.",
            "parameters": {"type": "OBJECT", "properties": {
                "action": {"type": "STRING", "enum": ["record_start", "record_stop", "record_pause", "record_resume", "stream_start", "stream_stop", "scene", "scenes", "mute", "unmute", "replay_save", "virtualcam_start", "virtualcam_stop", "status"]},
                "value": {"type": "STRING", "description": "Nome da cena (scene) ou da fonte de áudio (mute/unmute; vazio = microfone)."}
            }, "required": ["action"]}
        }),
        json!({
            "name": "whatsapp_call_control",
            "description": "Controla a ligação do WhatsApp em andamento: encerrar (hangup), atender (accept), recusar (decline) ou ligar/desligar o microfone da ligação (mute). Use SEMPRE esta ferramenta para ligações; nunca media_control.",
            "parameters": {"type": "OBJECT", "properties": {
                "action": {"type": "STRING", "enum": ["hangup", "accept", "decline", "mute"]}
            }, "required": ["action"]}
        }),
        json!({
            "name": "whatsapp_nickname",
            "description": "Guarda (ou apaga) um apelido para um contato do WhatsApp: 'quando eu falar Carol é a Carolina Braga'. Depois, o apelido funciona em mandar, ler e ligar.",
            "parameters": {"type": "OBJECT", "properties": {
                "nick": {"type": "STRING", "description": "O apelido que o usuário vai falar (ex.: 'Carol')."},
                "contact": {"type": "STRING", "description": "O nome do contato no WhatsApp (ex.: 'Carolina Braga')."},
                "remove": {"type": "BOOLEAN", "description": "true para apagar o apelido."}
            }, "required": ["nick"]}
        }),
        json!({
            "name": "whatsapp_watch",
            "description": "Liga (ou desliga) o aviso de mensagens novas de um contato do WhatsApp: quando ele mandar mensagem, o Jarvis avisa e pergunta se pode ler. Ex.: 'me avisa quando a Carol mandar mensagem'.",
            "parameters": {"type": "OBJECT", "properties": {
                "contact": {"type": "STRING", "description": "Nome ou apelido do contato."},
                "remove": {"type": "BOOLEAN", "description": "true para parar de avisar."}
            }, "required": ["contact"]}
        }),
        json!({
            "name": "whatsapp_unread",
            "description": "Lista as conversas do WhatsApp com mensagens não lidas (nome, quantas e a última). Só quando o usuário perguntar.",
            "parameters": {"type": "OBJECT", "properties": {}}
        }),
    ]
}

/// Extra rules for the system prompt.
pub const PROMPT_RULES: &str = r#"
8. Gmail: SÓ use `gmail_search`/`gmail_read`/`gmail_draft` quando o usuário pedir explicitamente algo sobre e-mail. Nunca mencione, conte ou resuma e-mails por iniciativa própria. Para responder/escrever, crie o rascunho com `gmail_draft`; com send=true o envio fica aguardando Permitir — avise isso.
9. WhatsApp: SÓ use as ferramentas de WhatsApp quando o usuário pedir explicitamente; nunca leia, conte ou comente mensagens por iniciativa própria. Enviar: chame `whatsapp_send` ("manda pra mim" → to "eu"); ele só PREPARA — repita ao usuário a pessoa e o texto exato e pergunte "Posso enviar?". Se o usuário confirmar, chame `whatsapp_send` de novo com os mesmos dados; se pedir mudança, chame com o texto novo e pergunte de novo. Ler uma conversa: `whatsapp_read`. "Tenho mensagem nova?": `whatsapp_unread`. Escreva a mensagem como o usuário escreveria e resuma o que leu em poucas frases.
12. Comandos personalizados: "quando eu falar X, abre A, B e C" / "cria um comando X que abre…" → `jarvis_action` kind "routine", value X (a frase exata que o usuário vai falar), extra2 com os programas separados por vírgula como ele falou. Depois, falar X roda a rotina sozinha. Diga o que ficou salvo e avise o que não foi achado. Apelidos de contato ("quando eu falar Carol é a Carolina Braga"): `whatsapp_nickname`; nas ferramentas de WhatsApp pode usar o apelido direto. "Me avisa quando a Carol mandar mensagem": `whatsapp_watch` (o Jarvis avisa, pergunta se pode ler e depois o que responder).
11. Programas do PC: para usar um programa, abra com `open_target` (se não estiver aberto), aja com `app_type` (digitar/atalhos) ou `app_click` (botões pelo nome) e confira com `app_read`, dizendo o resultado. Ex.: "calcula 125 vezes 8 na calculadora" → open_target "calc", app_type app "calculadora" text "125*8=", app_read "calculadora" → diga o valor. Antes de algo que apaga, envia, compra ou fecha sem salvar, pergunte ao usuário. Arquivos de texto: `file_read` antes de editar e `file_write` para salvar (a versão anterior fica guardada; avise onde).
10. "Bom dia", "como está meu dia", "o que tenho hoje": chame `daily_briefing` e responda em 2-4 frases naturais.
11. Arquivos: `file_search` e depois `file_open`. Notas/Obsidian: `notes_search`, `notes_read`, `notes_write` ("anota isso…").
12. Som: `audio_output` para trocar fone/caixa; `mic_mute` para o microfone; `set_volume` para volume. Internet lenta/caiu: `internet_status`.
13. Mídia: `media_control` funciona em qualquer app tocando (YouTube, VLC, Netflix…), não só no Spotify.
14. "Abre/abri X" (abri é como a transcrição escreve "abre"): para PROGRAMAS chame `open_target` só com o nome do app (ex.: "OpenBot", "Obsidian", "Steam") — ele acha o atalho no menu Iniciar. Nunca abra notas, .md ou arquivos soltos para atender "abre X" a menos que o usuário peça um arquivo. Responda sobre o app que ele pediu, não sobre o pedido anterior."#;

pub fn status_line(name: &str, args: &Value) -> Option<(String, String)> {
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    Some(match name {
        "gmail_search" => ("Lendo seus e-mails…".into(), s("query")),
        "gmail_read" => ("Abrindo o e-mail…".into(), String::new()),
        "gmail_draft" => ("Escrevendo o e-mail…".into(), s("subject")),
        "daily_briefing" => ("Montando o resumo do dia…".into(), String::new()),
        "file_search" => ("Procurando arquivos…".into(), s("query")),
        "file_open" => ("Abrindo arquivo…".into(), s("path")),
        "app_read" => ("Lendo a janela…".into(), s("app")),
        "app_type" => ("Digitando…".into(), s("app")),
        "app_click" => ("Clicando…".into(), s("target")),
        "app_list" => ("Vendo as janelas abertas…".into(), String::new()),
        "file_read" => ("Lendo o arquivo…".into(), s("path")),
        "file_write" => ("Editando o arquivo…".into(), s("path")),
        "notes_search" | "notes_read" => ("Lendo suas notas…".into(), s("query")),
        "notes_write" => ("Anotando no Obsidian…".into(), s("title")),
        "audio_output" => ("Trocando a saída de som…".into(), s("device")),
        "mic_mute" => ("Microfone…".into(), String::new()),
        "internet_status" => ("Testando a internet…".into(), String::new()),
        "whatsapp_send" => ("Preparando mensagem…".into(), s("to")),
        "whatsapp_read" => ("Lendo o WhatsApp…".into(), s("chat")),
        "whatsapp_unread" => ("Vendo o WhatsApp…".into(), String::new()),
        "whatsapp_watch" => ("Configurando o aviso…".into(), s("contact")),
        "whatsapp_call_control" => ("Na ligação…".into(), s("action")),
        "obs_control" => ("No OBS…".into(), s("action")),
        "whatsapp_nickname" => ("Guardando o apelido…".into(), s("nick")),
        "whatsapp_call" => ("Preparando a ligação…".into(), s("to")),
        _ => return None,
    })
}

/// The day's facts, in plain lines (the AI or the notch turns them into speech).
pub async fn briefing_facts() -> Vec<String> {
    let mut lines = Vec::new();
    let now = chrono::Local::now();
    lines.push(format!("Agora: {}", crate::google_calendar::now_context()));
    let city = crate::settings::load().jarvis_weather_city;
    if !city.trim().is_empty() {
        match crate::weather::now(&city).await {
            Ok(w) => lines.push(format!(
                "Clima em {}: {} graus, {}, máxima {} e mínima {}, {}% de chance de chuva.",
                w.place,
                w.temp.round(),
                w.label.to_lowercase(),
                w.max.round(),
                w.min.round(),
                w.rain_chance
            )),
            Err(e) => lines.push(format!("Clima indisponível: {e}")),
        }
    }
    if crate::google_calendar::connected() {
        match crate::google_calendar::events(crate::google_calendar::today(), 1).await {
            Ok(evs) if evs.is_empty() => lines.push("Agenda de hoje: nenhum compromisso.".into()),
            Ok(evs) => lines.push(format!("Agenda de hoje:\n{}", crate::google_calendar::describe(&evs, false))),
            Err(e) => lines.push(format!("Agenda indisponível: {e}")),
        }
    }
    let data = crate::jarvis_store::load_data();
    let tasks: Vec<_> = data.tasks.iter().filter(|t| !t.done).map(|t| t.title.clone()).collect();
    if !tasks.is_empty() {
        lines.push(format!("Tarefas pendentes ({}): {}", tasks.len(), tasks.iter().take(5).cloned().collect::<Vec<_>>().join("; ")));
    }
    let hm = now.format("%H:%M").to_string();
    let reminders: Vec<_> = data.reminders.iter().filter(|r| !r.done && r.time.as_str() >= hm.as_str()).map(|r| format!("{} às {}", r.title, r.time)).collect();
    if !reminders.is_empty() {
        lines.push(format!("Lembretes de hoje: {}", reminders.join("; ")));
    }
    lines
}

/// A spoken "bom dia" without the AI: instant, from the same facts.
pub async fn briefing_speech(user: &str) -> String {
    let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(9);
    let greet = match hour {
        5..=11 => "Bom dia",
        12..=17 => "Boa tarde",
        _ => "Boa noite",
    };
    let mut parts = vec![if user.trim().is_empty() { format!("{greet}!") } else { format!("{greet}, {}!", user.trim()) }];
    let city = crate::settings::load().jarvis_weather_city;
    if !city.trim().is_empty() {
        if let Ok(w) = crate::weather::now(&city).await {
            let rain = if w.rain_chance >= 50 { format!(" Leve guarda-chuva: {}% de chance de chuva.", w.rain_chance) } else { String::new() };
            parts.push(format!("Agora faz {} graus, {}, com máxima de {}.{rain}", w.temp.round(), w.label.to_lowercase(), w.max.round()));
        }
    }
    if crate::google_calendar::connected() {
        if let Ok(evs) = crate::google_calendar::events(crate::google_calendar::today(), 1).await {
            let upcoming: Vec<_> = evs.iter().filter(|e| e.all_day || e.end_local() > chrono::Local::now().naive_local()).collect();
            match upcoming.len() {
                0 => parts.push("Sua agenda está livre hoje.".into()),
                n => {
                    let first = upcoming[0];
                    let when = if first.all_day { "o dia todo".to_string() } else { format!("às {}", first.start_hm()) };
                    parts.push(format!(
                        "Você tem {} {} hoje{}: o primeiro é {} {}.",
                        n,
                        if n == 1 { "compromisso" } else { "compromissos" },
                        "",
                        first.title,
                        when
                    ));
                }
            }
        }
    }
    let data = crate::jarvis_store::load_data();
    let pending = data.tasks.iter().filter(|t| !t.done).count();
    if pending > 0 {
        parts.push(format!("E {} {} pendente{}.", pending, if pending == 1 { "tarefa" } else { "tarefas" }, if pending == 1 { "" } else { "s" }));
    }
    parts.join(" ")
}

#[tauri::command]
pub async fn jarvis_briefing() -> String {
    let user = crate::jarvis_store::load_data().user_name;
    briefing_speech(&user).await
}


/// Runs a confirmed e-mail / WhatsApp. None = not one of these kinds.
pub async fn run_confirmed(kind: &str, command: &str) -> Option<Result<String, String>> {
    match kind {
        GMAIL_SEND => Some(crate::gmail::send_draft(command).await),
        WHATSAPP => {
            let v: Value = serde_json::from_str(command).unwrap_or(Value::Null);
            let (to, msg) = (v["to"].as_str().unwrap_or(""), v["message"].as_str().unwrap_or(""));
            match crate::whatsapp::send(to, msg).await {
                // Not linked here but OpenBot is set up: it sends instead.
                Err(e) if e == crate::whatsapp::NEED_LOGIN && openbot_ready() => {
                    let order = format!("Envie esta mensagem no WhatsApp para {to}, exatamente com este texto, sem alterar:\n{msg}");
                    Some(crate::jarvis_brain::send_to_openbot_webhook(order, None, Some("orquestrador".into())).await.map(|_| format!("Mensagem para {to} enviada ao OpenBot.")))
                }
                other => Some(other),
            }
        }
        _ => None,
    }
}

/// OpenBot's webhook has its secret (the WhatsApp fallback can be used).
fn openbot_ready() -> bool {
    crate::secrets::get("openbot-webhook-secret").is_some() || crate::jarvis_accounts::read_env_or_hkcu("OPENBOT_WEBHOOK_SECRET").is_some()
}

fn ok(out: String) -> (bool, String, String) {
    let first = out.lines().next().unwrap_or("").chars().take(140).collect::<String>();
    (true, out, first)
}

fn err(e: String) -> (bool, String, String) {
    (false, e.clone(), format!("Erro: {e}"))
}

/// Runs one of these tools. None = not a tool of this module.
pub async fn run(
    _app: &AppHandle,
    name: &str,
    args: &Value,
    queue: &mut (dyn FnMut(&str, &str, &str) -> crate::jarvis_brain::PendingAction + Send),
) -> Option<(bool, String, String)> {
    let s = |k: &str| args[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let r = match name {
        "gmail_search" => crate::gmail::list(&s("query").unwrap_or_default(), args["max"].as_u64().unwrap_or(10) as u32).await.map(|m| crate::gmail::describe(&m)),
        "gmail_read" => match s("id") {
            Some(id) => crate::gmail::read(&id).await.map(|m| format!("De: {}\nPara: {}\nAssunto: {}\nData: {}\n\n{}", m.item.from, m.to, m.item.subject, m.item.date, m.body)),
            None => Err("Falta o id do e-mail.".into()),
        },
        "gmail_draft" => {
            let body = s("body").unwrap_or_default();
            match crate::gmail::draft(s("to").as_deref(), s("subject").as_deref(), &body, s("reply_to_id").as_deref()).await {
                Ok(d) if args["send"].as_bool().unwrap_or(false) => {
                    let desc = format!("Enviar e-mail para {} — {}", d.to, d.subject);
                    queue(GMAIL_SEND, &d.id, &desc);
                    Ok(format!("Rascunho criado e AGUARDANDO o usuário tocar em Permitir para enviar a {} (assunto: {}). Não diga que já enviou.", d.to, d.subject))
                }
                Ok(d) => Ok(format!("Rascunho salvo no Gmail para {} (assunto: {}). Não foi enviado.", d.to, d.subject)),
                Err(e) => Err(e),
            }
        }
        "daily_briefing" => Ok(briefing_facts().await.join("\n")),
        "file_search" => {
            let q = s("query").unwrap_or_default();
            tauri::async_runtime::spawn_blocking(move || crate::file_search::search(&q, 15)).await.unwrap_or_else(|e| Err(e.to_string())).map(|h| crate::file_search::describe(&h))
        }
        "file_open" => crate::file_search::open(&s("path").unwrap_or_default(), args["reveal"].as_bool().unwrap_or(false)),
        "app_read" | "app_type" | "app_click" | "app_list" | "file_read" | "file_write" => {
            let (name, a) = (name.to_string(), args.clone());
            tauri::async_runtime::spawn_blocking(move || {
                let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
                match name.as_str() {
                    "app_read" => crate::pc_control::read(&s("app")),
                    "app_type" => crate::pc_control::type_into(&s("app"), &s("text"), &s("keys"), &s("field")),
                    "app_click" => crate::pc_control::click(&s("app"), &s("target")),
                    "app_list" => Ok(crate::pc_control::list()),
                    "file_read" => crate::pc_control::read_file(&s("path")),
                    _ => crate::pc_control::write_file(&s("path"), &s("mode"), &s("content"), &s("find"), &s("replace")),
                }
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
        "notes_search" => {
            let q = s("query").unwrap_or_default();
            tauri::async_runtime::spawn_blocking(move || crate::notes::search(&q, 10)).await.unwrap_or_else(|e| Err(e.to_string())).map(|h| crate::notes::describe(&h))
        }
        "notes_read" => crate::notes::read(&s("path").unwrap_or_default()),
        "notes_write" => crate::notes::write(&s("title").unwrap_or_else(|| "Jarvis".into()), &s("text").unwrap_or_default(), s("folder").as_deref()).map(|p| format!("Anotado em {p}")),
        "audio_output" => {
            let d = s("device").unwrap_or_default();
            tauri::async_runtime::spawn_blocking(move || crate::audio::switch_output(&d)).await.unwrap_or_else(|e| Err(e.to_string()))
        }
        "mic_mute" => {
            let mute = args["mute"].as_bool().unwrap_or(true);
            tauri::async_runtime::spawn_blocking(move || crate::audio::set_mic_mute(mute))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
                .map(|_| if mute { "Microfone mudo.".to_string() } else { "Microfone ligado.".to_string() })
        }
        "internet_status" => {
            let ping = tauri::async_runtime::spawn_blocking(crate::pc_monitor::ping_once).await.ok().flatten();
            match ping {
                None => Ok("Sem internet: não consegui alcançar a Cloudflare nem o Google.".into()),
                Some(ms) if args["speed_test"].as_bool().unwrap_or(false) => match crate::pc_monitor::speed_test().await {
                    Ok(mbps) => Ok(format!("Internet ok: ping {ms} ms, download {:.0} Mbit/s.", mbps)),
                    Err(e) => Ok(format!("Ping {ms} ms, mas o teste de velocidade falhou: {e}")),
                },
                Some(ms) => Ok(format!("Internet ok: ping {ms} ms.")),
            }
        }
        "whatsapp_send" => match (s("to"), s("message")) {
            // Confirmed in the conversation, not with a button: the first call
            // holds it and Jarvis asks "Posso enviar?" (whatsapp.rs).
            (Some(to), Some(msg)) => match crate::whatsapp::prepare(&to, &msg) {
                crate::whatsapp::Prepared::Ask(say) => Ok(say),
                crate::whatsapp::Prepared::Send(to, msg) => crate::whatsapp::send(&to, &msg).await,
            },
            _ => Err("Falta o contato ou a mensagem.".into()),
        },
        "whatsapp_read" => match s("chat") {
            Some(chat) => crate::whatsapp::read(&chat, args["count"].as_u64().unwrap_or(10) as usize).await,
            None => Err("Qual conversa?".into()),
        },
        "whatsapp_unread" => crate::whatsapp::unread().await,
        "obs_control" => crate::obs::act(&s("action").unwrap_or_default(), &s("value").unwrap_or_default()).await,
        "whatsapp_call_control" => match s("action").as_deref() {
            Some(a @ ("hangup" | "accept" | "decline" | "mute")) => crate::whatsapp::call_action(a).await.map(|c| {
                if c.state == "none" {
                    "Feito. Não há mais ligação.".to_string()
                } else {
                    format!("Feito. Ligação: {}{}.", c.state, if c.mic_off { ", microfone desligado" } else { "" })
                }
            }),
            _ => Err("Ação de ligação desconhecida.".into()),
        },
        "whatsapp_watch" => match s("contact") {
            Some(c) => crate::whatsapp::watch(&c, !args["remove"].as_bool().unwrap_or(false)),
            None => Err("Qual contato?".into()),
        },
        "whatsapp_nickname" => match (s("nick"), s("contact")) {
            (Some(nick), _) if args["remove"].as_bool().unwrap_or(false) => crate::jarvis_store::remove_nickname(&nick)
                .map(|had| if had { format!("Apaguei o apelido \"{nick}\".") } else { format!("Não havia o apelido \"{nick}\".") }),
            (Some(nick), Some(contact)) => crate::whatsapp::add_nickname(&nick, &contact).await,
            _ => Err("Falta o apelido ou o contato.".into()),
        },
        "whatsapp_call" => match s("to") {
            Some(to) => Ok(format!(
                "Ligação PREPARADA, ainda não feita. Responda ao usuário exatamente: {}",
                crate::whatsapp::prepare_call(&to, args["video"].as_bool().unwrap_or(false))
            )),
            None => Err("Para quem?".into()),
        },
        _ => return None,
    };
    Some(match r {
        Ok(o) => ok(o),
        Err(e) => err(e),
    })
}
