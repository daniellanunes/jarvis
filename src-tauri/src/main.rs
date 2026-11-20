// OpenJarvis runs without a console window: Mochi is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn is_process_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Admin rights, asked once (see elevation.rs):
/// - already elevated: go on — and, in a release build, make sure the task that
///   avoids the next prompt exists and points at this exe;
/// - started through that task: go on (never loop back to it);
/// - otherwise: start through the task, silently elevated, and exit;
/// - no task yet (or a dev build): the UAC prompt, as before.
#[cfg(windows)]
fn ensure_uac_elevation() {
    use openjarvis_lib::elevation;
    let args: Vec<String> = std::env::args().collect();
    let release = !cfg!(debug_assertions);
    let exe = std::env::current_exe().ok();

    if is_process_elevated() {
        if release {
            if let Some(exe) = exe.clone() {
                // Off the startup path: schtasks takes a moment.
                std::thread::spawn(move || {
                    if !elevation::task_ready(&exe) {
                        let _ = elevation::register(&exe);
                    }
                });
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--no-elevate" || a == elevation::FROM_TASK) {
        return;
    }
    if release {
        if let Some(exe) = exe.as_deref() {
            if elevation::task_ready(exe) && elevation::run_task() {
                std::process::exit(0);
            }
        }
    }
    let Some(exe) = exe else { return };

    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: isize,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: isize,
        lp_id_list: *mut core::ffi::c_void,
        lp_class: *const u16,
        hkey_class: isize,
        dw_hot_key: u32,
        h_icon_or_monitor: isize,
        h_process: isize,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(p_exec_info: *mut ShellExecuteInfoW) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn WaitForSingleObject(h_handle: isize, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: isize) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let mut args: Vec<String> = std::env::args()
        .skip(1)
        .map(|a| {
            if a.contains(' ') {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a
            }
        })
        .collect();
    args.push("--no-elevate".to_string());
    let params = args.join(" ");

    let verb_w = to_wide("runas");
    let exe_w = to_wide(&exe.to_string_lossy());
    let params_w = to_wide(&params);

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SW_HIDE: i32 = 0;
    const INFINITE: u32 = 0xFFFF_FFFF;

    let mut sei = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: 0,
        lp_verb: verb_w.as_ptr(),
        lp_file: exe_w.as_ptr(),
        lp_parameters: params_w.as_ptr(),
        lp_directory: std::ptr::null(),
        n_show: SW_HIDE,
        h_inst_app: 0,
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: 0,
        dw_hot_key: 0,
        h_icon_or_monitor: 0,
        h_process: 0,
    };

    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok != 0 {
        if sei.h_process != 0 {
            if cfg!(debug_assertions) {
                // Keep parent alive so `tauri dev` does not kill the Vite dev server.
                unsafe { WaitForSingleObject(sei.h_process, INFINITE) };
            }
            unsafe { CloseHandle(sei.h_process) };
        }
        std::process::exit(0);
    }
}

fn main() {
    #[cfg(windows)]
    {
        // Run by the installer / uninstaller (already elevated), then exit.
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--register-elevation") {
            let code = match openjarvis_lib::elevation::on_install() {
                Ok(()) => 0,
                Err(_) => 1,
            };
            std::process::exit(code);
        }
        if args.iter().any(|a| a == "--unregister-elevation") {
            openjarvis_lib::elevation::on_uninstall();
            std::process::exit(0);
        }
        ensure_uac_elevation();
    }
    openjarvis_lib::run()
}
