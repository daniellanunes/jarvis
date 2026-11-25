//! Alerts on the PC: at each popup reminder of the Google Agenda events
//! (e.g. "10 min antes") and at the time of the Jarvis's own reminders
//! (Organizar → Lembretes), the island gets `jarvis-alert` — toast, sound and
//! voice. Google also notifies on the phone; this is the PC side.

use chrono::{Local, NaiveDateTime};
use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub title: String,
    pub body: String,
    /// Rings like an alarm clock (the user asked for "alarme" / "despertador").
    pub alarm: bool,
}

/// Marks an event created as an alarm (in its description).
pub const ALARM_MARK: &str = "⏰ Alarme (Jarvis)";

/// Set when the Jarvis creates or changes an event: the agenda is read again
/// on the next check instead of within 2 minutes.
pub static REFRESH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether an event asks to ring like an alarm.
pub fn is_alarm(title: &str, description: &str) -> bool {
    let t = crate::file_search::fold(title);
    description.contains(ALARM_MARK) || ["despertador", "alarme", "acordar"].iter().any(|w| t.contains(w))
}

/// An alert counts as due for 3 minutes after its time (the check runs every
/// 20 s; a PC that was asleep does not get a pile of old alerts).
const WINDOW_MIN: i64 = 3;

pub fn due(fire_at: NaiveDateTime, now: NaiveDateTime) -> bool {
    now >= fire_at && now < fire_at + chrono::Duration::minutes(WINDOW_MIN)
}

/// "em 10 minutos", "agora", "em 1 hora".
pub fn lead_text(minutes: i64) -> String {
    match minutes {
        0 => "agora".into(),
        1 => "em 1 minuto".into(),
        m if m < 60 => format!("em {m} minutos"),
        60 => "em 1 hora".into(),
        m if m % 60 == 0 && m < 1440 => format!("em {} horas", m / 60),
        1440 => "amanhã".into(),
        m => format!("em {} min", m),
    }
}

fn event_start(ev: &crate::google_calendar::CalEvent) -> Option<NaiveDateTime> {
    if ev.all_day {
        return chrono::NaiveDate::parse_from_str(&ev.start, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0);
    }
    chrono::DateTime::parse_from_rfc3339(&ev.start).ok().map(|d| d.with_timezone(&Local).naive_local())
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut fired: HashSet<String> = HashSet::new();
        let mut events: Vec<crate::google_calendar::CalEvent> = Vec::new();
        let mut fetched: Option<Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let now = Local::now().naive_local();
            let today = now.format("%Y-%m-%d").to_string();
            let mut alerts: Vec<Alert> = Vec::new();

            // Google Agenda: today and the next 2 days (reminders up to a day before).
            if crate::google_calendar::connected() {
                let asked = REFRESH.swap(false, std::sync::atomic::Ordering::SeqCst);
                if asked || fetched.is_none_or(|t| t.elapsed() > Duration::from_secs(120)) {
                    if let Ok(list) = crate::google_calendar::events(crate::google_calendar::today(), 3).await {
                        events = list;
                    }
                    fetched = Some(Instant::now());
                }
                for ev in &events {
                    let Some(start) = event_start(ev) else { continue };
                    for &m in &ev.reminders {
                        let key = format!("{}|{}|{}", ev.key, ev.start, m);
                        if !fired.contains(&key) && due(start - chrono::Duration::minutes(m), now) {
                            fired.insert(key);
                            let when = if ev.all_day { "hoje".to_string() } else { format!("{} · {}", lead_text(m), start.format("%H:%M")) };
                            alerts.push(Alert { title: ev.title.clone(), body: when, alarm: is_alarm(&ev.title, &ev.description) });
                        }
                    }
                }
            } else {
                events.clear();
            }

            // Organizar → Lembretes ("HH:MM", today).
            for r in crate::jarvis_store::load_data().reminders.iter().filter(|r| !r.done) {
                let Ok(t) = chrono::NaiveTime::parse_from_str(r.time.trim(), "%H:%M") else { continue };
                let key = format!("rem|{}|{today}", r.id);
                if !fired.contains(&key) && due(now.date().and_time(t), now) {
                    fired.insert(key);
                    alerts.push(Alert { title: r.title.clone(), body: format!("lembrete das {}", r.time.trim()), alarm: is_alarm(&r.title, "") });
                }
            }

            for a in alerts {
                crate::log::line(format!("alert: {} ({})", a.title, a.body));
                // Full screen / OBS on air: told afterwards.
                if let Some(why) = crate::dnd::quiet().await {
                    crate::dnd::hold_alert(&format!("{}, {}", a.title, a.body), why);
                    continue;
                }
                let _ = app.emit_to(crate::island::WINDOW_LABEL, "jarvis-alert", a);
            }
            if fired.len() > 2000 {
                fired.clear();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn alerts_fire_on_time_and_not_hours_later() {
        let fire = at("2026-11-20 10:35");
        assert!(!due(fire, at("2026-11-20 10:34")));
        assert!(due(fire, at("2026-11-20 10:35")));
        assert!(due(fire, at("2026-11-20 10:37")));
        assert!(!due(fire, at("2026-11-20 12:00")));
    }

    #[test]
    fn lead_times_read_naturally() {
        assert_eq!(lead_text(0), "agora");
        assert_eq!(lead_text(10), "em 10 minutos");
        assert_eq!(lead_text(60), "em 1 hora");
        assert_eq!(lead_text(120), "em 2 horas");
    }
}

#[cfg(test)]
mod alarm_tests {
    #[test]
    fn alarms_are_told_from_reminders() {
        assert!(super::is_alarm("Prova da faculdade", "Estudar\n⏰ Alarme (Jarvis)"));
        assert!(super::is_alarm("Despertador da prova", ""));
        assert!(!super::is_alarm("Jogo do Barcelona", "assistir"));
    }
}
