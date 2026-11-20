// Notification-area icon: Open, Settings, Quit — the three the Jarvis notch
// answers. (Weekly recap, Wardrobe and Pause belonged to the old island
// screen, which is no longer loaded; the separate settings window fails to
// start, so Settings opens the notch's own Ajustes tab.)

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::i18n::{n_, t};
use crate::island::WINDOW_LABEL;

/// The menu's items, id and English label, in order. The labels are shown in
/// the interface language (i18n.rs) and follow it when it changes.
const ITEMS: [(&str, &str); 3] = [
    ("open", n_("Open OpenJarvis")),
    ("settings", n_("Settings…")),
    ("quit", n_("Quit")),
];

/// Kept so a language change relabels the menu without rebuilding the tray.
struct Items(Vec<(&'static str, MenuItem<Wry>)>);

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut items = Vec::new();
    for (id, label) in ITEMS {
        items.push((label, MenuItem::with_id(app, id, t(label), true, None::<&str>)?));
    }
    let [open, settings, quit] = [0, 1, 2].map(|i| &items[i].1);
    let sep = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[open, settings, &sep, quit])?;
    app.manage(Items(items));

    let mut builder = TrayIconBuilder::with_id("openjarvis")
        .tooltip("OpenJarvis")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            // "open" → Início, "settings" → Ajustes, both in the notch (notch_app.ts).
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// Relabels the menu in the current language.
pub fn retitle(app: &AppHandle) {
    let Some(items) = app.try_state::<Items>() else { return };
    for (label, item) in &items.0 {
        let _ = item.set_text(t(label));
    }
}
