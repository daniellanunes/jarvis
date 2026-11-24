//! Media of any app — YouTube in the browser, VLC, Netflix, the Spotify app —
//! through Windows' media sessions (the ones the volume flyout shows). A
//! thread reads the current session every 2 s and emits `media-now` to the
//! island when it changes; the notch shows it when Spotify is not playing.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaNow {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// "Chrome", "VLC", "Spotify"…
    pub app: String,
    pub album: String,
}

/// "Microsoft.ZuneVideo_8wekyb3d8bbwe!Microsoft.ZuneVideo" / "chrome.exe" → a short name.
pub fn app_label(aumid: &str) -> String {
    let lower = aumid.to_lowercase();
    let known = [
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("spotify", "Spotify"),
        ("vlc", "VLC"),
        ("zunemusic", "Media Player"),
        ("zunevideo", "Filmes e TV"),
        ("netflix", "Netflix"),
        ("primevideo", "Prime Video"),
        ("disney", "Disney+"),
        ("deezer", "Deezer"),
        ("tidal", "Tidal"),
        ("youtube", "YouTube"),
    ];
    if let Some((_, l)) = known.iter().find(|(k, _)| lower.contains(k)) {
        return l.to_string();
    }
    let base = aumid.rsplit(['\\', '/', '!']).next().unwrap_or(aumid);
    base.trim_end_matches(".exe").to_string()
}

#[cfg(windows)]
mod win {
    use super::{app_label, MediaNow};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };

    thread_local!(static RO: () = unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    });

    /// When Windows' media service last failed: it is not asked again for 30 s
    /// (a failing call can take seconds, and a voice command would wait on it).
    static LAST_FAIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

    fn manager() -> Option<Manager> {
        RO.with(|_| ());
        if LAST_FAIL.lock().ok().and_then(|f| *f).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(30)) {
            return None;
        }
        match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => Some(m),
            Err(e) => {
                crate::log::line(format!("media sessions unavailable (retry in 30 s): {e}"));
                if let Ok(mut f) = LAST_FAIL.lock() {
                    *f = Some(std::time::Instant::now());
                }
                None
            }
        }
    }

    /// The playing session first (several can be open), else the system's current one.
    fn pick(m: &Manager) -> Option<Session> {
        if let Ok(list) = m.GetSessions() {
            for s in list {
                let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|st| st == Status::Playing).unwrap_or(false);
                if playing {
                    return Some(s);
                }
            }
        }
        m.GetCurrentSession().ok()
    }

    pub fn now() -> Option<MediaNow> {
        let m = manager()?;
        let s = pick(&m)?;
        let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|st| st == Status::Playing).unwrap_or(false);
        Some(MediaNow {
            title: props.Title().map(|t| t.to_string()).unwrap_or_default(),
            artist: props.Artist().map(|t| t.to_string()).unwrap_or_default(),
            playing,
            app: s.SourceAppUserModelId().map(|a| app_label(&a.to_string())).unwrap_or_default(),
            album: props.AlbumTitle().map(|t| t.to_string()).unwrap_or_default(),
        })
    }

    /// The session showing this title (the one on the notch's card), if any.
    fn by_title(m: &Manager, title: &str) -> Option<Session> {
        let want = title.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        m.GetSessions().ok()?.into_iter().find(|s| {
            s.TryGetMediaPropertiesAsync()
                .ok()
                .and_then(|op| op.get().ok())
                .and_then(|p| p.Title().ok())
                .is_some_and(|t| t.to_string().trim().to_lowercase() == want)
        })
    }

    pub fn control(action: &str) -> Result<String, String> {
        control_for(action, "")
    }

    /// Like `control`, on the session of `title` when there is one.
    pub fn control_for(action: &str, title: &str) -> Result<String, String> {
        let m = manager().ok_or("Nenhuma mídia aberta.")?;
        let s = by_title(&m, title).or_else(|| pick(&m)).ok_or("Nenhuma mídia aberta.")?;
        let op = match action {
            "play" => s.TryPlayAsync(),
            "pause" => s.TryPauseAsync(),
            "next" => s.TrySkipNextAsync(),
            "previous" | "prev" => s.TrySkipPreviousAsync(),
            _ => s.TryTogglePlayPauseAsync(),
        }
        .map_err(|e| e.to_string())?;
        let ok = op.get().map_err(|e| e.to_string())?;
        let app = s.SourceAppUserModelId().map(|a| app_label(&a.to_string())).unwrap_or_default();
        if ok {
            Ok(format!("{app}: {}", match action {
                "play" => "tocando",
                "pause" => "pausado",
                "next" => "próxima",
                "previous" | "prev" => "anterior",
                _ => "play/pause",
            }))
        } else {
            Err(format!("{app} não aceitou o comando."))
        }
    }
}

#[cfg(not(windows))]
mod win {
    use super::MediaNow;
    pub fn now() -> Option<MediaNow> {
        None
    }
    pub fn control(_: &str) -> Result<String, String> {
        Err("Só no Windows".into())
    }
    pub fn control_for(_: &str, _: &str) -> Result<String, String> {
        Err("Só no Windows".into())
    }
}

pub use win::now;

/// Play/pause/next/previous on whatever app is playing.
pub fn control(action: &str) -> Result<String, String> {
    win::control(action)
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<MediaNow> = None;
        loop {
            let cur = now();
            if cur != last {
                let _ = app.emit_to(crate::island::WINDOW_LABEL, "media-now", cur.clone());
                last = cur;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

#[tauri::command]
pub async fn media_now() -> Option<MediaNow> {
    tauri::async_runtime::spawn_blocking(now).await.ok().flatten()
}

/// The notch's player buttons: the card's own session (by its title), else
/// the Spotify account, else the keyboard's media keys.
#[tauri::command]
pub async fn player_control(action: String, title: Option<String>) -> Result<String, String> {
    let (a, t) = (action.clone(), title.unwrap_or_default());
    let direct = tauri::async_runtime::spawn_blocking(move || win::control_for(&a, &t)).await.map_err(|e| e.to_string())?;
    match direct {
        Ok(msg) => Ok(msg),
        Err(e) => {
            crate::log::line(format!("player: sessão de mídia não respondeu ({e}); tentando outro caminho"));
            if let Some(msg) = crate::jarvis_brain::api_media_control(&action).await {
                return Ok(msg);
            }
            crate::jarvis_pc::jarvis_media_control(action)
        }
    }
}

#[tauri::command]
pub async fn media_control(action: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || control(&action)).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_get_short_app_names() {
        assert_eq!(app_label("Chrome"), "Chrome");
        assert_eq!(app_label("Spotify.exe"), "Spotify");
        assert_eq!(app_label("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(app_label("C:\\Apps\\foobar2000.exe"), "foobar2000");
    }

    /// `cargo test reads_this_pc_media -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn reads_this_pc_media() {
        println!("{:?}", now());
    }
}

#[cfg(all(test, windows))]
mod live {
    /// Toggles the current track twice (back to how it was).
    /// `cargo test --lib media_sessions::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_card_buttons_reach_the_track_shown() {
        let before = super::now();
        println!("antes: {before:?}");
        let title = before.as_ref().map(|m| m.title.clone()).unwrap_or_default();
        println!("play/pause: {:?}", super::win::control_for("play_pause", &title));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        println!("depois: {:?}", super::now());
        println!("de volta: {:?}", super::win::control_for("play_pause", &title));
    }
}
