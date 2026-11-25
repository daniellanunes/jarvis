//! Automatic "não perturbe": while a game or anything else is in full screen,
//! or OBS is recording / live, the Jarvis keeps quiet. WhatsApp messages from
//! watched contacts and reminders are held, and told in one go afterwards
//! ("Enquanto você jogava: a Bia mandou 3 mensagens. Quer que eu leia?").
//! Ajustes → Notch → "Não perturbe automático" (on by default).

use serde_json::json;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, PartialEq)]
enum Held {
    /// A watched WhatsApp contact wrote (how many messages).
    Whatsapp(String, u32),
    /// A reminder or calendar alert.
    Alert(String),
}

static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());
/// Why it was quiet ("jogava", "gravava"), for the summary.
static WHY: Mutex<String> = Mutex::new(String::new());

/// Why the Jarvis should keep quiet now. None = it can speak.
pub async fn quiet() -> Option<&'static str> {
    if !crate::settings::load().jarvis_auto_dnd {
        return None;
    }
    if tauri::async_runtime::spawn_blocking(crate::pc_monitor::fullscreen_app).await.ok().flatten().is_some() {
        return Some("jogava ou estava em tela cheia");
    }
    if crate::obs::on_air().await {
        return Some("gravava ou estava ao vivo");
    }
    None
}

fn note_why(why: &str) {
    *WHY.lock().unwrap_or_else(|e| e.into_inner()) = why.to_string();
}

/// A watched contact's message, held for later.
pub fn hold_whatsapp(contact: &str, count: u32, why: &str) {
    note_why(why);
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(Held::Whatsapp(_, n)) = held.iter_mut().find(|h| matches!(h, Held::Whatsapp(c, _) if c == contact)) {
        *n += count.max(1);
    } else {
        held.push(Held::Whatsapp(contact.to_string(), count.max(1)));
    }
    crate::log::line(format!("não perturbe: mensagem de {contact} guardada para depois"));
}

/// A reminder, held for later.
pub fn hold_alert(text: &str, why: &str) {
    note_why(why);
    HELD.lock().unwrap_or_else(|e| e.into_inner()).push(Held::Alert(text.to_string()));
    crate::log::line(format!("não perturbe: lembrete guardado ({text})"));
}

/// What to say once it is over; and the single contact to offer reading, if any.
fn summary(held: &[Held], why: &str) -> (String, Option<(String, u32)>) {
    let wa: Vec<(String, u32)> = held.iter().filter_map(|h| if let Held::Whatsapp(c, n) = h { Some((c.clone(), *n)) } else { None }).collect();
    let alerts: Vec<String> = held.iter().filter_map(|h| if let Held::Alert(t) = h { Some(t.clone()) } else { None }).collect();
    let mut parts = Vec::new();
    for (c, n) in &wa {
        parts.push(if *n > 1 { format!("{c} mandou {n} mensagens") } else { format!("{c} mandou uma mensagem") });
    }
    if alerts.len() == 1 {
        parts.push(format!("lembrete: {}", alerts[0]));
    } else if alerts.len() > 1 {
        parts.push(format!("{} lembretes: {}", alerts.len(), alerts.join("; ")));
    }
    let list = match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!("{} e {}", parts[..parts.len() - 1].join(", "), parts[parts.len() - 1]),
    };
    let one = (wa.len() == 1).then(|| wa[0].clone());
    let mut text = format!("Enquanto você {why}: {list}.");
    if one.is_some() {
        text.push_str(" Quer que eu leia?");
    }
    (text, one)
}

/// Every few seconds while something is held: once the quiet is over, it is
/// told — read out, with "Quer que eu leia?" when one contact wrote.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if HELD.lock().unwrap_or_else(|e| e.into_inner()).is_empty() || quiet().await.is_some() {
                continue;
            }
            let held = std::mem::take(&mut *HELD.lock().unwrap_or_else(|e| e.into_inner()));
            let why = WHY.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let (text, one) = summary(&held, if why.is_empty() { "estava ocupado" } else { &why });
            crate::log::line(format!("não perturbe acabou: {text}"));
            match one {
                Some((contact, n)) => {
                    crate::whatsapp::offer_reading(&contact, n);
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, "whatsapp-incoming", json!({ "contact": contact, "text": text }));
                }
                None => {
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, "jarvis-say", text);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_says_what_was_missed() {
        let (t, one) = summary(&[Held::Whatsapp("Bia".into(), 3)], "jogava");
        assert_eq!(t, "Enquanto você jogava: Bia mandou 3 mensagens. Quer que eu leia?");
        assert_eq!(one, Some(("Bia".into(), 3)));
        let (t, one) = summary(&[Held::Whatsapp("Bia".into(), 1), Held::Whatsapp("Ana".into(), 2), Held::Alert("Pagar a conta".into())], "gravava");
        assert_eq!(t, "Enquanto você gravava: Bia mandou uma mensagem, Ana mandou 2 mensagens e lembrete: Pagar a conta.");
        assert_eq!(one, None);
    }
}

#[cfg(test)]
mod probe {
    /// `cargo test --lib dnd::probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn quiet_now() {
        println!("tela cheia: {:?}", crate::pc_monitor::fullscreen_app());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        println!("não perturbe: {:?}", rt.block_on(super::quiet()));
    }
}
