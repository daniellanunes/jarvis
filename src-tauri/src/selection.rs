//! The text selected in any app: Ctrl+C is sent to the foreground window and
//! the clipboard read back, then the user's clipboard is put back as it was.
//! Used by the translate hotkey (Ctrl+Alt+K).

#[cfg(windows)]
mod win {
    use std::time::Duration;
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_C, VK_LMENU, VK_MENU, VK_RMENU,
    };

    const CF_UNICODETEXT: u32 = 13;

    fn open() -> bool {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    pub fn read_text() -> Option<String> {
        if !open() {
            return None;
        }
        let out = unsafe {
            GetClipboardData(CF_UNICODETEXT).ok().and_then(|h| {
                let g = HGLOBAL(h.0);
                let p = GlobalLock(g) as *const u16;
                if p.is_null() {
                    return None;
                }
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
                let _ = GlobalUnlock(g);
                Some(s)
            })
        };
        unsafe {
            let _ = CloseClipboard();
        }
        out
    }

    pub fn write_text(text: &str) {
        if !open() {
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
                let p = GlobalLock(g) as *mut u16;
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                    let _ = GlobalUnlock(g);
                    let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
        }
    }

    pub fn copy_selection() -> Option<String> {
        let before = read_text();
        let seq = unsafe { GetClipboardSequenceNumber() };
        // The hotkey's own Alt is still down: Ctrl+Alt+C would not copy. Wait
        // for the user to let go (up to 1 s), then lift Alt in case.
        for _ in 0..50 {
            let alt = unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as u16 & 0x8000 != 0;
            if !alt {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let inputs = [key(VK_LMENU, true), key(VK_RMENU, true), key(VK_CONTROL, false), key(VK_C, false), key(VK_C, true), key(VK_CONTROL, true)];
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(20));
            if unsafe { GetClipboardSequenceNumber() } != seq {
                break;
            }
        }
        if unsafe { GetClipboardSequenceNumber() } == seq {
            return None; // nothing was selected
        }
        std::thread::sleep(Duration::from_millis(30));
        let text = read_text();
        if let Some(b) = before {
            write_text(&b);
        }
        text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    }
}

/// The selected text in the foreground app, or None when nothing is selected.
#[tauri::command]
pub async fn jarvis_selected_text() -> Option<String> {
    #[cfg(windows)]
    return tauri::async_runtime::spawn_blocking(win::copy_selection).await.ok().flatten();
    #[cfg(not(windows))]
    None
}

/// Puts text on the clipboard (the "Copiar" of a translation).
#[tauri::command]
pub fn jarvis_copy_text(text: String) {
    #[cfg(windows)]
    win::write_text(&text);
    #[cfg(not(windows))]
    let _ = text;
}
