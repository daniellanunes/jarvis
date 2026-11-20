//! Audio for the Meu PC tab, the notch and voice: which outputs exist (fone,
//! caixa…), switching the default one, master volume, and the microphone's
//! mute. Windows has no public call to change the default device; like every
//! audio switcher, this uses the shell's IPolicyConfig.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    pub outputs: Vec<AudioDevice>,
    pub volume: u32,
    pub muted: bool,
    /// None when there is no microphone.
    pub mic_muted: Option<bool>,
    pub mic_name: String,
}

/// "Fone" matches "Fones de ouvido (Realtek…)", "caixa" a speaker, etc.
pub fn pick_device<'a>(devices: &'a [AudioDevice], wanted: &str) -> Option<&'a AudioDevice> {
    let w = wanted.trim().to_lowercase();
    if w.is_empty() {
        return None;
    }
    let aliases: &[(&[&str], &[&str])] = &[
        (&["fone", "headphone", "headset", "ouvido"], &["fone", "headphone", "headset", "ouvido", "earphone", "airpods", "buds"]),
        (&["caixa", "alto-falante", "alto falante", "speaker", "som"], &["alto-falante", "speaker", "caixa", "realtek", "som"]),
        (&["monitor", "tela", "hdmi", "tv"], &["hdmi", "displayport", "monitor", "tv", "nvidia high definition", "amd high definition"]),
    ];
    let name_has = |d: &AudioDevice, words: &[&str]| {
        let n = d.name.to_lowercase();
        words.iter().any(|x| n.contains(x))
    };
    devices
        .iter()
        .find(|d| d.name.to_lowercase().contains(&w))
        .or_else(|| aliases.iter().find(|(keys, _)| keys.iter().any(|k| w.contains(k))).and_then(|(_, words)| devices.iter().find(|d| name_has(d, words))))
}

#[cfg(windows)]
#[allow(non_snake_case)]
mod win {
    use super::{AudioDevice, AudioState};
    use std::ffi::c_void;
    use windows::core::{interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT, PCWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, eMultimedia, eRender, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

    /// The shell's undocumented default-device setter (used by every switcher).
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, fmt: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, default: i32, fmt: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, def: *mut i64, min: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    /// Runs `f` on a fresh COM thread (callers may be on any runtime thread).
    fn com<T: Send + 'static>(f: impl FnOnce(&IMMDeviceEnumerator) -> Result<T, String> + Send + 'static) -> Result<T, String> {
        std::thread::spawn(move || unsafe {
            let init = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let r = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("Áudio indisponível: {e}"))
                .and_then(|en| f(&en));
            if init {
                CoUninitialize();
            }
            r
        })
        .join()
        .map_err(|_| "Falha na thread de áudio".to_string())?
    }

    unsafe fn id_of(d: &IMMDevice) -> String {
        match d.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            }
            Err(_) => String::new(),
        }
    }

    #[cfg(test)]
    pub fn name_of_pub(d: &IMMDevice) -> String {
        unsafe { name_of(d) }
    }

    unsafe fn name_of(d: &IMMDevice) -> String {
        let Ok(store) = d.OpenPropertyStore(STGM_READ) else { return String::new() };
        let Ok(v) = store.GetValue(&PKEY_Device_FriendlyName) else { return String::new() };
        match PropVariantToStringAlloc(&v) {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            }
            Err(_) => String::new(),
        }
    }

    pub fn state() -> Result<AudioState, String> {
        com(|en| unsafe {
            let default_id = en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| id_of(&d)).unwrap_or_default();
            let mut outputs = Vec::new();
            if let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                for i in 0..col.GetCount().unwrap_or(0) {
                    if let Ok(d) = col.Item(i) {
                        let id = id_of(&d);
                        outputs.push(AudioDevice { default: id == default_id, name: name_of(&d), id });
                    }
                }
            }
            let mut st = AudioState { outputs, ..Default::default() };
            if let Ok(d) = en.GetDefaultAudioEndpoint(eRender, eMultimedia) {
                if let Ok(v) = d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    st.volume = (v.GetMasterVolumeLevelScalar().unwrap_or(0.0) * 100.0).round() as u32;
                    st.muted = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                }
            }
            if let Ok(d) = en.GetDefaultAudioEndpoint(eCapture, eCommunications) {
                st.mic_name = name_of(&d);
                if let Ok(v) = d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    st.mic_muted = v.GetMute().ok().map(|b| b.as_bool());
                }
            }
            Ok(st)
        })
    }

    pub fn set_default(id: String) -> Result<(), String> {
        com(move |_| unsafe {
            let pc: IPolicyConfig = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL).map_err(|e| format!("Troca de saída indisponível: {e}"))?;
            let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            for role in [eConsole, eMultimedia, eCommunications] {
                pc.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role).ok().map_err(|e| format!("Não consegui trocar a saída: {e}"))?;
            }
            Ok(())
        })
    }

    pub fn set_mic_mute(mute: bool) -> Result<(), String> {
        com(move |en| unsafe {
            let mut done = false;
            // Every active microphone, so no app keeps listening on another one.
            if let Ok(col) = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
                for i in 0..col.GetCount().unwrap_or(0) {
                    if let Ok(v) = col.Item(i).and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)) {
                        done |= v.SetMute(mute, std::ptr::null()).is_ok();
                    }
                }
            }
            if done { Ok(()) } else { Err("Nenhum microfone encontrado.".into()) }
        })
    }

    pub fn set_mute(mute: bool) -> Result<(), String> {
        com(move |en| unsafe {
            let d = en.GetDefaultAudioEndpoint(eRender, eMultimedia).map_err(|e| e.to_string())?;
            let v = d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
            v.SetMute(mute, std::ptr::null()).map_err(|e| e.to_string())
        })
    }

}

#[cfg(not(windows))]
mod win {
    use super::AudioState;
    pub fn state() -> Result<AudioState, String> {
        Err("Só no Windows".into())
    }
    pub fn set_default(_: String) -> Result<(), String> {
        Err("Só no Windows".into())
    }
    pub fn set_mic_mute(_: bool) -> Result<(), String> {
        Err("Só no Windows".into())
    }
    pub fn set_mute(_: bool) -> Result<(), String> {
        Err("Só no Windows".into())
    }
}

pub use win::{set_mic_mute, set_mute, state};

/// Makes the output whose name matches `wanted` ("fone", "caixa", a name) the default.
pub fn switch_output(wanted: &str) -> Result<String, String> {
    let st = state()?;
    let dev = pick_device(&st.outputs, wanted)
        .or_else(|| {
            // "troca a saída" with no name: the next one.
            let w = wanted.trim();
            if w.is_empty() || w == "proxima" || w == "próxima" || w == "outra" {
                let i = st.outputs.iter().position(|d| d.default).map(|i| (i + 1) % st.outputs.len().max(1)).unwrap_or(0);
                st.outputs.get(i)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            let names: Vec<_> = st.outputs.iter().map(|d| d.name.as_str()).collect();
            format!("Não achei a saída \"{wanted}\". Saídas: {}", names.join(", "))
        })?
        .clone();
    win::set_default(dev.id.clone())?;
    Ok(format!("Som saindo em: {}", dev.name))
}

#[tauri::command]
pub async fn audio_state() -> Result<AudioState, String> {
    tauri::async_runtime::spawn_blocking(state).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn audio_set_output(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || win::set_default(id)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn audio_set_mic_mute(mute: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_mic_mute(mute)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn audio_set_mute(mute: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_mute(mute)).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> AudioDevice {
        AudioDevice { id: name.into(), name: name.into(), default: false }
    }

    #[test]
    fn spoken_names_find_the_output() {
        let list = vec![dev("Alto-falantes (Realtek(R) Audio)"), dev("Fones de ouvido (HyperX Cloud II)"), dev("LG HDR 4K (AMD High Definition Audio)")];
        assert_eq!(pick_device(&list, "fone").unwrap().name, "Fones de ouvido (HyperX Cloud II)");
        assert_eq!(pick_device(&list, "caixa de som").unwrap().name, "Alto-falantes (Realtek(R) Audio)");
        assert_eq!(pick_device(&list, "monitor").unwrap().name, "LG HDR 4K (AMD High Definition Audio)");
        assert_eq!(pick_device(&list, "hyperx").unwrap().name, "Fones de ouvido (HyperX Cloud II)");
        assert!(pick_device(&list, "bluetooth").is_none());
    }

    /// `cargo test reads_this_pc_audio -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn reads_this_pc_audio() {
        println!("{:#?}", state().unwrap());
    }
}

#[cfg(all(test, windows))]
mod capture_probe {
    /// `cargo test lists_microphones -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lists_microphones() {
        use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let console = en.GetDefaultAudioEndpoint(eCapture, eConsole).map(|d| super::win::name_of_pub(&d)).unwrap_or_default();
            let comms = en.GetDefaultAudioEndpoint(eCapture, eCommunications).map(|d| super::win::name_of_pub(&d)).unwrap_or_default();
            println!("padrão (apps/navegador): {console}");
            println!("padrão (comunicações): {comms}");
            let col = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).unwrap();
            for i in 0..col.GetCount().unwrap() {
                println!("  microfone: {}", super::win::name_of_pub(&col.Item(i).unwrap()));
            }
        }
    }
}
