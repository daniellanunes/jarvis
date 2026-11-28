// API keys live in the Windows Credential Manager or, on Linux, the Secret
// Service (GNOME Keyring, KWallet) — never on disk and never in the front end — the island can only ask whether a key is present.

use keyring::Entry;

const SERVICE: &str = "com.openjarvis.desktop";

/// Every key OpenJarvis may store. Anything outside this list is refused.
pub const KNOWN_KEYS: &[&str] = &[
    "groq-api-key",
    "anthropic-api-key",
    "openai-api-key",
    "google-api-key",
    "openrouter-api-key",
    "openai-compatible-key",
    "n8n-url",
    "n8n-api-key",
    "vercel-token",
    "github-token",
    "obs-password",
    "stripe-api-key",
    "resend-api-key",
    "notion-api-key",
    "calcom-api-key",
    "claude-oauth-token",
    "gemini-oauth-token",
    "openbot-webhook-secret",
    "gemini-oauth-client-id",
    "openai-oauth-client-id",
    "claude-oauth-client-id",
    "gemini-oauth-client-secret",
    "spotify-refresh-token",
    "google-calendar-refresh-token",
    "google-calendar-email",
];

/// The keychain service before the app was renamed from Coucou: secrets saved
/// there are still read, and copied to SERVICE the first time they are used.
const LEGACY_SERVICE: &str = concat!("fr.louisraille.", "cou", "cou");

/// Runs a Credential Manager operation; when Windows says this process has no
/// vault (the Jarvis started elevated by its scheduled task runs in a logon
/// session without the user's credential set: ERROR_NO_SUCH_LOGON_SESSION),
/// the same operation runs again in the signed-in user's session.
fn with_vault<T: Send + 'static>(op: impl Fn() -> keyring::Result<T> + Send + Sync + 'static) -> keyring::Result<T> {
    let op = std::sync::Arc::new(op);
    match op() {
        Err(keyring::Error::NoStorageAccess(e)) | Err(keyring::Error::PlatformFailure(e)) => {
            #[cfg(windows)]
            {
                let again = op.clone();
                if let Some(result) = as_user::run(move || again()) {
                    return result;
                }
            }
            Err(keyring::Error::NoStorageAccess(e))
        }
        other => other,
    }
}

/// Borrowing the signed-in user's own session (Explorer's token, same user,
/// same Windows session) for one Credential Manager call, on a thread of its
/// own that goes back to the process's identity right after.
#[cfg(windows)]
mod as_user {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{ImpersonateLoggedOnUser, RevertToSelf, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    /// Explorer of this Windows session.
    fn explorer_pid() -> Option<u32> {
        unsafe {
            let mut mine = 0u32;
            ProcessIdToSessionId(GetCurrentProcessId(), &mut mine).ok()?;
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case("explorer.exe") {
                    let mut sid = u32::MAX;
                    if ProcessIdToSessionId(e.th32ProcessID, &mut sid).is_ok() && sid == mine {
                        found = Some(e.th32ProcessID);
                        break;
                    }
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    /// `f` as the signed-in user; None when that identity could not be taken.
    pub fn run<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        std::thread::spawn(move || unsafe {
            let pid = explorer_pid()?;
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut token = HANDLE::default();
            let opened = OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE, &mut token).is_ok();
            let _ = CloseHandle(process);
            if !opened {
                return None;
            }
            let as_them = ImpersonateLoggedOnUser(token).is_ok();
            let out = if as_them { Some(f()) } else { None };
            let _ = RevertToSelf();
            let _ = CloseHandle(token);
            if out.is_some() {
                crate::log::line("cofre: usado pela sessão do usuário (Jarvis aberto pela tarefa de administrador)");
            }
            out
        })
        .join()
        .ok()
        .flatten()
    }

    #[cfg(test)]
    mod tests {
        /// The borrowed session writes, reads and deletes a credential.
        #[test]
        fn the_users_session_reaches_the_vault() {
            use keyring::Entry;
            let key = "jarvis-selftest-vault";
            let wrote = super::run(move || Entry::new(super::super::SERVICE, key).and_then(|e| e.set_password("ok")));
            assert!(matches!(wrote, Some(Ok(()))), "{wrote:?}");
            let read = super::run(move || Entry::new(super::super::SERVICE, key).and_then(|e| e.get_password()));
            assert_eq!(read.and_then(|r| r.ok()).as_deref(), Some("ok"));
            let gone = super::run(move || Entry::new(super::super::SERVICE, key).and_then(|e| e.delete_credential()));
            assert!(matches!(gone, Some(Ok(()))));
        }
    }
}

fn entry(key: &str) -> Option<Entry> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    Entry::new(SERVICE, key).ok()
}

/// Reads `user` from the keychain, falling back to the pre-rename service and
/// migrating what it finds there. For entries outside KNOWN_KEYS too (per
/// account tokens in jarvis_accounts.rs).
pub fn get_raw(user: &str) -> Option<String> {
    let u = user.to_string();
    if let Some(v) = with_vault(move || Entry::new(SERVICE, &u)?.get_password()).ok().filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let u = user.to_string();
    let v = with_vault(move || Entry::new(LEGACY_SERVICE, &u)?.get_password()).ok().filter(|v| !v.is_empty())?;
    let _ = set_raw(user, &v);
    Some(v)
}

pub fn set_raw(user: &str, value: &str) -> Result<(), String> {
    let (u, v) = (user.to_string(), value.to_string());
    with_vault(move || Entry::new(SERVICE, &u)?.set_password(&v)).map_err(|e| e.to_string())
}

/// Removes `user` from both the current and the pre-rename service.
pub fn delete_raw(user: &str) {
    for service in [SERVICE, LEGACY_SERVICE] {
        let u = user.to_string();
        let _ = with_vault(move || Entry::new(service, &u)?.delete_credential());
    }
}

pub fn get(key: &str) -> Option<String> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    get_raw(key)
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    let (k, v) = (key.to_string(), value.to_string());
    if v.is_empty() {
        let _ = with_vault(move || Entry::new(SERVICE, &k)?.delete_credential());
        return Ok(());
    }
    with_vault(move || Entry::new(SERVICE, &k)?.set_password(&v)).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    // Also forget a pre-rename copy, or `get` would bring the value back.
    let k = key.to_string();
    let _ = with_vault(move || Entry::new(LEGACY_SERVICE, &k)?.delete_credential());
    let k = key.to_string();
    match with_vault(move || Entry::new(SERVICE, &k)?.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn present(key: &str) -> bool {
    get(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::KNOWN_KEYS;

    /// Every key the code reads or writes must be in KNOWN_KEYS, or the
    /// keychain silently refuses it ("unknown key …").
    #[test]
    fn every_key_used_in_the_code_is_known() {
        let sources = [
            include_str!("jarvis_brain.rs"),
            include_str!("jarvis_accounts.rs"),
            include_str!("spotify_api.rs"),
            include_str!("google_calendar.rs"),
            include_str!("integrations.rs"),
            include_str!("openai_compat.rs"),
            include_str!("claude.rs"),
            include_str!("obs.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for call in ["secrets::get(\"", "secrets::set(\"", "secrets::clear(\"", "secrets::present(\"", "_KEY: &str = \""] {
                for part in src.split(call).skip(1) {
                    let key = part.split('"').next().unwrap_or("");
                    if key.contains('-') && !KNOWN_KEYS.contains(&key) {
                        missing.push(key.to_string());
                    }
                }
            }
        }
        assert!(missing.is_empty(), "keys missing from KNOWN_KEYS: {missing:?}");
    }
}
