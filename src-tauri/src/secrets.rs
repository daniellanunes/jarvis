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
    let current = Entry::new(SERVICE, user).ok()?;
    if let Some(v) = current.get_password().ok().filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let legacy = Entry::new(LEGACY_SERVICE, user).ok()?;
    let v = legacy.get_password().ok().filter(|v| !v.is_empty())?;
    let _ = current.set_password(&v);
    Some(v)
}

pub fn set_raw(user: &str, value: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE, user).map_err(|e| format!("Keyring indisponível: {e}"))?;
    entry.set_password(value).map_err(|e| e.to_string())
}

/// Removes `user` from both the current and the pre-rename service.
pub fn delete_raw(user: &str) {
    for service in [SERVICE, LEGACY_SERVICE] {
        if let Ok(e) = Entry::new(service, user) {
            let _ = e.delete_credential();
        }
    }
}

pub fn get(key: &str) -> Option<String> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    get_raw(key)
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    // Also forget a pre-rename copy, or `get` would bring the value back.
    if let Ok(legacy) = Entry::new(LEGACY_SERVICE, key) {
        let _ = legacy.delete_credential();
    }
    match entry.delete_credential() {
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
