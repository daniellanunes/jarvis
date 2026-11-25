//! Live PC readings for the notch: CPU, RAM, GPU and disk usage, and the
//! frame rate of the game (or other GPU-heavy program) in the foreground.
//!
//! - CPU from GetSystemTimes deltas, RAM from GlobalMemoryStatusEx, disk C:
//!   from GetDiskFreeSpaceExW — no PowerShell, a sample costs microseconds.
//! - GPU from the "GPU Engine(*engtype_3D)\Utilization Percentage" performance
//!   counters (what Task Manager shows), which also tell which process uses it.
//! - FPS: like PresentMon, frames are counted from the DXGI and Direct3D 9 ETW
//!   providers (Present events, per process). An ETW session needs admin rights
//!   (the app asks for them at start); without them FPS is just not shown.
//!
//! A thread samples once a second while a notch widget wants it and emits
//! `pc-metrics` to the island window; otherwise it sleeps.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static PC_ON: AtomicBool = AtomicBool::new(false);
static FPS_ON: AtomicBool = AtomicBool::new(false);
static NET_ON: AtomicBool = AtomicBool::new(false);
/// Last ping in ms (u32::MAX = offline, 0 = not measured yet).
static PING_MS: AtomicU32 = AtomicU32::new(0);
/// Last temperatures in tenths of °C (0 = unknown).
static CPU_TEMP: AtomicU32 = AtomicU32::new(0);
static GPU_TEMP: AtomicU32 = AtomicU32::new(0);
static SENSORS: AtomicBool = AtomicBool::new(false);
/// Move the notch to the other monitor during fullscreen games.
static GAME_OTHER: AtomicBool = AtomicBool::new(true);

pub fn set_game_other_screen(on: bool) {
    GAME_OTHER.store(on, Ordering::Relaxed);
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub cpu: f32,
    pub ram: f32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    /// None when the GPU counters are unavailable.
    pub gpu: Option<f32>,
    pub disk: f32,
    pub disk_free_gb: f32,
    pub disk_total_gb: f32,
    pub cpu_name: String,
    pub gpu_name: String,
    /// Frames per second of the foreground game/program, when it renders.
    pub fps: Option<u32>,
    pub fps_app: String,
    /// False when FPS was asked for but the ETW session could not start.
    pub fps_available: bool,
    /// Internet: round trip to Cloudflare/Google in ms; None = offline.
    pub ping_ms: Option<u32>,
    pub online: bool,
    pub net_on: bool,
    /// °C, when LibreHardwareMonitor is running (see temperatures()).
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub sensors: bool,
    /// The notch moved to another monitor for this game.
    pub moved_for_game: bool,
    /// Physical top-left of the monitor of a fullscreen game (internal).
    #[serde(skip)]
    pub game_monitor: Option<(i32, i32)>,
}

/// Turns the readings on or off (from the notch settings).
pub fn configure(pc: bool, fps: bool, net: bool) {
    PC_ON.store(pc, Ordering::Relaxed);
    NET_ON.store(net, Ordering::Relaxed);
    FPS_ON.store(fps, Ordering::Relaxed);
    #[cfg(windows)]
    if fps {
        win::start_fps_trace();
    }
}

/// Round trip of a TCP handshake — no ICMP needed (works without admin and
/// through most firewalls).
pub fn ping_once() -> Option<u32> {
    use std::net::{SocketAddr, TcpStream};
    let targets: [SocketAddr; 2] = [([1, 1, 1, 1], 443).into(), ([8, 8, 8, 8], 443).into()];
    targets.iter().find_map(|t| {
        let at = std::time::Instant::now();
        TcpStream::connect_timeout(t, Duration::from_millis(1500)).ok().map(|_| at.elapsed().as_millis().max(1) as u32)
    })
}

/// Download speed in Mbit/s from Cloudflare's speed test (about 10 MB).
pub async fn speed_test() -> Result<f32, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let at = std::time::Instant::now();
    let bytes = client
        .get("https://speed.cloudflare.com/__down?bytes=10000000")
        .send()
        .await
        .map_err(|e| format!("Sem internet ({e})"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let secs = at.elapsed().as_secs_f32().max(0.05);
    Ok(bytes.len() as f32 * 8.0 / 1_000_000.0 / secs)
}

#[tauri::command]
pub async fn net_speed_test() -> Result<f32, String> {
    speed_test().await
}

fn spawn_side_readers() {
    // Ping every 5 s while the internet widget is on.
    std::thread::spawn(|| loop {
        if NET_ON.load(Ordering::Relaxed) {
            PING_MS.store(ping_once().unwrap_or(u32::MAX), Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(5));
        } else {
            PING_MS.store(0, Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    // Temperatures every 10 s while the PC widget is on.
    #[cfg(windows)]
    std::thread::spawn(|| loop {
        if PC_ON.load(Ordering::Relaxed) {
            SENSORS.store(win::sensor_app_running(), Ordering::Relaxed);
            let (c, g) = win::temperatures();
            CPU_TEMP.store(c.map(|v| (v * 10.0) as u32).unwrap_or(0), Ordering::Relaxed);
            GPU_TEMP.store(g.map(|v| (v * 10.0) as u32).unwrap_or(0), Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(10));
        } else {
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[cfg_attr(not(windows), allow(dead_code))]
fn add_side_readings(m: &mut Metrics) {
    m.net_on = NET_ON.load(Ordering::Relaxed);
    let p = PING_MS.load(Ordering::Relaxed);
    m.online = p != u32::MAX;
    m.ping_ms = (p != 0 && p != u32::MAX).then_some(p);
    let t = |a: &AtomicU32| Some(a.load(Ordering::Relaxed)).filter(|v| *v > 0).map(|v| v as f32 / 10.0);
    m.cpu_temp = t(&CPU_TEMP);
    m.gpu_temp = t(&GPU_TEMP);
    m.sensors = SENSORS.load(Ordering::Relaxed);
}

pub fn spawn(app: AppHandle) {
    spawn_side_readers();
    std::thread::spawn(move || {
        #[cfg(windows)]
        let mut sampler = win::Sampler::new();
        #[cfg(windows)]
        let mut game_misses = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            let (pc, fps, net) = (PC_ON.load(Ordering::Relaxed), FPS_ON.load(Ordering::Relaxed), NET_ON.load(Ordering::Relaxed));
            if !pc && !fps && !net {
                continue;
            }
            #[cfg(windows)]
            {
                let mut m = sampler.sample(fps);
                add_side_readings(&mut m);
                // A game on screen: stay above it, and leave its monitor when
                // it is fullscreen (exclusive fullscreen hides every window).
                if m.fps.is_some() {
                    game_misses = 0;
                    crate::island::reassert_topmost(&app);
                    if GAME_OTHER.load(Ordering::Relaxed) && m.game_monitor.is_some() {
                        crate::island::set_game_screen(&app, m.game_monitor);
                    }
                } else if crate::island::game_screen_active() {
                    game_misses += 1;
                    // A few seconds without frames (menus, loading) before going back.
                    if game_misses >= 8 || !GAME_OTHER.load(Ordering::Relaxed) {
                        crate::island::set_game_screen(&app, None);
                    }
                }
                m.moved_for_game = crate::island::game_screen_active();
                let _ = app.emit_to(crate::island::WINDOW_LABEL, "pc-metrics", m);
            }
            #[cfg(not(windows))]
            let _ = &app;
        }
    });
}

/// "pid_1234_luid_0x…_engtype_3D" → 1234.
pub fn pid_of_instance(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("pid_")?;
    rest.split('_').next()?.parse().ok()
}

/// Programs that present frames all the time without being games.
pub fn not_a_game(exe: &str) -> bool {
    const SKIP: &[&str] = &[
        "explorer", "openjarvis", "msedgewebview2", "chrome", "msedge", "firefox", "brave", "opera",
        "discord", "spotify", "code", "windowsterminal", "dwm", "searchhost", "shellexperiencehost",
        "startmenuexperiencehost", "textinputhost", "applicationframehost",
    ];
    let e = exe.to_ascii_lowercase();
    SKIP.iter().any(|s| e == *s)
}

#[cfg(windows)]
mod win {
    use super::Metrics;
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    use std::time::Instant;
    use windows::core::{w, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, FILETIME, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W,
        PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    };
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::{
        GetSystemTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, GWL_STYLE, WS_CAPTION};

    const GB: f32 = 1024.0 * 1024.0 * 1024.0;

    // ── Frames, counted from ETW ──────────────────────────────────────────────
    const TRACE_NAME: &str = "OpenJarvisFps";
    const DXGI: &str = "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9"; // Microsoft-Windows-DXGI
    const D3D9: &str = "783ACA0A-790E-4D7F-8451-AA850511C6B9"; // Microsoft-Windows-D3D9

    /// Presents per process since the last sample.
    static PRESENTS: LazyLock<Mutex<HashMap<u32, u32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    static TRACE: LazyLock<Mutex<Option<ferrisetw::UserTrace>>> = LazyLock::new(|| Mutex::new(None));
    static TRACE_STATE: LazyLock<Mutex<Option<bool>>> = LazyLock::new(|| Mutex::new(None));

    fn count_present(pid: u32) {
        if let Ok(mut m) = PRESENTS.lock() {
            *m.entry(pid).or_insert(0) += 1;
        }
    }

    /// Starts the ETW session once (needs admin). Safe to call repeatedly.
    pub fn start_fps_trace() {
        let mut state = TRACE_STATE.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_some() {
            return;
        }
        // A session left over by a crash would block ours.
        let _ = ferrisetw::trace::stop_trace_by_name(TRACE_NAME);
        let dxgi = ferrisetw::provider::Provider::by_guid(DXGI)
            .level(5)
            .add_callback(|rec: &ferrisetw::EventRecord, _: &ferrisetw::schema_locator::SchemaLocator| {
                if rec.event_id() == 42 {
                    count_present(rec.process_id()); // Present_Start
                }
            })
            .build();
        let d3d9 = ferrisetw::provider::Provider::by_guid(D3D9)
            .level(5)
            .add_callback(|rec: &ferrisetw::EventRecord, _: &ferrisetw::schema_locator::SchemaLocator| {
                if rec.event_id() == 1 {
                    count_present(rec.process_id()); // Present_Start
                }
            })
            .build();
        match ferrisetw::UserTrace::new().named(TRACE_NAME.to_string()).enable(dxgi).enable(d3d9).start_and_process() {
            Ok(trace) => {
                *TRACE.lock().unwrap_or_else(|e| e.into_inner()) = Some(trace);
                *state = Some(true);
            }
            Err(e) => {
                crate::log::line(format!("fps: ETW session unavailable ({e:?}) — run as administrator"));
                *state = Some(false);
            }
        }
    }

    fn fps_available() -> bool {
        TRACE_STATE.lock().ok().and_then(|s| *s).unwrap_or(false)
    }

    // ── GPU counters ──────────────────────────────────────────────────────────
    struct Gpu {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    // The handles are only used from the sampler thread.
    unsafe impl Send for Gpu {}

    impl Gpu {
        fn open() -> Option<Self> {
            unsafe {
                let mut query = PDH_HQUERY::default();
                if PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }
                let mut counter = PDH_HCOUNTER::default();
                if PdhAddEnglishCounterW(query, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut counter) != 0 {
                    return None;
                }
                PdhCollectQueryData(query);
                Some(Self { query, counter })
            }
        }

        /// (total %, % per process)
        fn read(&self) -> Option<(f32, HashMap<u32, f32>)> {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let mut size = 0u32;
                let mut count = 0u32;
                let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
                if r != PDH_MORE_DATA || size == 0 {
                    return Some((0.0, HashMap::new()));
                }
                // Items followed by their names: one aligned block.
                let mut buf: Vec<u64> = vec![0; (size as usize).div_ceil(8) + 1];
                let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                    return None;
                }
                let mut total = 0.0f32;
                let mut per = HashMap::new();
                for i in 0..count as usize {
                    let item = &*items.add(i);
                    if item.FmtValue.CStatus != 0 {
                        continue;
                    }
                    let v = item.FmtValue.Anonymous.doubleValue as f32;
                    total += v;
                    if let Some(pid) = item.szName.to_string().ok().as_deref().and_then(super::pid_of_instance) {
                        *per.entry(pid).or_insert(0.0) += v;
                    }
                }
                Some((total.min(100.0), per))
            }
        }
    }

    fn ft(f: FILETIME) -> u64 {
        ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
    }

    fn exe_name(pid: u32) -> String {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(h);
            if !ok {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        }
    }

    /// The foreground window's process, whether that window fills its monitor,
    /// and that monitor's physical top-left.
    fn foreground() -> Option<(u32, bool, (i32, i32))> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let full = GetMonitorInfoW(mon, &mut mi).as_bool()
                && wr.left <= mi.rcMonitor.left
                && wr.top <= mi.rcMonitor.top
                && wr.right >= mi.rcMonitor.right
                && wr.bottom >= mi.rcMonitor.bottom;
            Some((pid, full, (mi.rcMonitor.left, mi.rcMonitor.top)))
        }
    }

    /// The program in true full screen in front (a game, a video): its file
    /// name. A maximized window keeps its title bar, so it does not count.
    pub fn fullscreen_app() -> Option<String> {
        let (pid, full, _) = foreground()?;
        if !full {
            return None;
        }
        unsafe {
            let style = GetWindowLongW(GetForegroundWindow(), GWL_STYLE) as u32;
            if style & WS_CAPTION.0 == WS_CAPTION.0 {
                return None;
            }
        }
        let exe = exe_name(pid);
        const NOT: &[&str] = &[
            "explorer", "openjarvis", "msedgewebview2", "dwm", "searchhost", "shellexperiencehost", "startmenuexperiencehost",
            "textinputhost", "lockapp", "applicationframehost",
        ];
        let e = exe.to_ascii_lowercase();
        (!e.is_empty() && !NOT.iter().any(|n| e == *n)).then_some(exe)
    }

    /// CPU and GPU model names, read once (WMI through PowerShell, off the UI).
    fn hardware_names() -> (String, String) {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_Processor | Select -First 1).Name; (Get-CimInstance Win32_VideoController | Sort-Object AdapterRAM -Descending | Select -First 1).Name",
            ])
            .creation_flags(0x0800_0000)
            .output();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        (lines.next().unwrap_or("Processador").to_string(), lines.next().unwrap_or("Placa de vídeo").to_string())
    }

    /// Whether LibreHardwareMonitor / OpenHardwareMonitor is running.
    pub fn sensor_app_running() -> bool {
        use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut found = false;
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
                if name.starts_with("librehardwaremonitor") || name.starts_with("openhardwaremonitor") {
                    found = true;
                    break;
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    /// (CPU °C, GPU °C) from LibreHardwareMonitor (or OpenHardwareMonitor) —
    /// the only way to read AMD/Intel package and GPU sensors without a kernel
    /// driver of our own. Windows' ACPI "thermal zone" is not the CPU (it read
    /// 17 °C here), so without one of them there is no temperature.
    pub fn temperatures() -> (Option<f32>, Option<f32>) {
        use std::os::windows::process::CommandExt;
        if !sensor_app_running() {
            return (None, None);
        }
        let script = r#"$ErrorActionPreference='SilentlyContinue'
foreach($ns in 'root/LibreHardwareMonitor','root/OpenHardwareMonitor'){
 $s = Get-CimInstance -Namespace $ns -ClassName Sensor | Where-Object SensorType -eq 'Temperature'
 if($s){
  $c = $s | Where-Object { $_.Identifier -match 'cpu' -and $_.Name -match 'Package|Tctl|Tdie' } | Select-Object -First 1
  if(-not $c){ $c = $s | Where-Object Identifier -match 'cpu' | Select-Object -First 1 }
  $g = $s | Where-Object { $_.Identifier -match 'gpu' -and $_.Name -match 'Core|Edge' } | Select-Object -First 1
  if(-not $g){ $g = $s | Where-Object Identifier -match 'gpu' | Select-Object -First 1 }
  "lhm;$($c.Value);$($g.Value)"; exit
 }
}"#;
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x0800_0000)
            .output();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        super::parse_temps(&text)
    }

    pub struct Sampler {
        cpu_prev: Option<(u64, u64)>,
        gpu: Option<Gpu>,
        gpu_tried: bool,
        names: Option<(String, String)>,
        last: Instant,
        /// Smoothed FPS for the shown app, so the number does not flicker.
        fps_smooth: Option<(u32, f32)>,
    }

    impl Sampler {
        pub fn new() -> Self {
            Self { cpu_prev: None, gpu: None, gpu_tried: false, names: None, last: Instant::now(), fps_smooth: None }
        }

        fn cpu(&mut self) -> f32 {
            let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
                return 0.0;
            }
            let (idle, total) = (ft(idle), ft(kernel) + ft(user)); // kernel time includes idle
            let pct = match self.cpu_prev {
                Some((pi, pt)) if total > pt => {
                    let dt = (total - pt) as f32;
                    ((dt - (idle - pi) as f32) / dt * 100.0).clamp(0.0, 100.0)
                }
                _ => 0.0,
            };
            self.cpu_prev = Some((idle, total));
            pct
        }

        pub fn sample(&mut self, want_fps: bool) -> Metrics {
            let names = self.names.get_or_insert_with(hardware_names).clone();
            let mut m = Metrics { cpu: self.cpu(), cpu_name: names.0, gpu_name: names.1, ..Default::default() };

            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if unsafe { GlobalMemoryStatusEx(&mut ms) }.is_ok() {
                m.ram = ms.dwMemoryLoad as f32;
                m.ram_total_gb = ms.ullTotalPhys as f32 / GB;
                m.ram_used_gb = (ms.ullTotalPhys - ms.ullAvailPhys) as f32 / GB;
            }
            let (mut free, mut total) = (0u64, 0u64);
            if unsafe { GetDiskFreeSpaceExW(w!("C:\\"), Some(&mut free), Some(&mut total), None) }.is_ok() && total > 0 {
                m.disk_total_gb = total as f32 / GB;
                m.disk_free_gb = free as f32 / GB;
                m.disk = (1.0 - free as f32 / total as f32) * 100.0;
            }

            if !self.gpu_tried {
                self.gpu_tried = true;
                self.gpu = Gpu::open();
            }
            let gpu = self.gpu.as_ref().and_then(Gpu::read);
            m.gpu = gpu.as_ref().map(|(t, _)| *t);

            let elapsed = self.last.elapsed().as_secs_f32().max(0.2);
            self.last = Instant::now();
            let presents = std::mem::take(&mut *PRESENTS.lock().unwrap_or_else(|e| e.into_inner()));
            m.fps_available = fps_available();
            if want_fps && m.fps_available {
                if let Some((pid, fullscreen, monitor)) = foreground() {
                    let frames = presents.get(&pid).copied().unwrap_or(0) as f32 / elapsed;
                    let gpu_use = gpu.as_ref().and_then(|(_, per)| per.get(&pid).copied()).unwrap_or(0.0);
                    let exe = exe_name(pid);
                    // A game, or another program working the GPU hard.
                    if frames >= 5.0 && (fullscreen || gpu_use >= 15.0) && !super::not_a_game(&exe) {
                        let smooth = match self.fps_smooth {
                            Some((p, v)) if p == pid => v * 0.5 + frames * 0.5,
                            _ => frames,
                        };
                        self.fps_smooth = Some((pid, smooth));
                        m.fps = Some(smooth.round() as u32);
                        m.fps_app = exe;
                        if fullscreen {
                            m.game_monitor = Some(monitor);
                        }
                    } else {
                        self.fps_smooth = None;
                    }
                }
            }
            m
        }
    }
}

/// The program in full screen in front, if any (for "não perturbe").
pub fn fullscreen_app() -> Option<String> {
    #[cfg(windows)]
    {
        win::fullscreen_app()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// "lhm;61.5;55" / "acpi;47.9;" → (CPU, GPU), dropping impossible values.
pub fn parse_temps(text: &str) -> (Option<f32>, Option<f32>) {
    let line = text.lines().last().unwrap_or("");
    let mut parts = line.split(';');
    let _kind = parts.next();
    let num = |s: Option<&str>| s.and_then(|v| v.trim().replace(',', ".").parse::<f32>().ok()).filter(|v| (5.0..125.0).contains(v));
    (num(parts.next()), num(parts.next()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_lines_parse() {
        assert_eq!(parse_temps("lhm;61,5;55"), (Some(61.5), Some(55.0)));
        assert_eq!(parse_temps("acpi;47.9;"), (Some(47.9), None));
        assert_eq!(parse_temps("acpi;-273.1;"), (None, None));
        assert_eq!(parse_temps(""), (None, None));
    }

    #[cfg(windows)]
    #[test]
    #[ignore]
    fn reads_this_pc_temperatures() {
        println!("{:?} ping {:?}", win::temperatures(), ping_once());
    }

    #[test]
    fn gpu_counter_instances_name_their_process() {
        assert_eq!(pid_of_instance("pid_1234_luid_0x00000000_0x0000D1B6_phys_0_eng_0_engtype_3D"), Some(1234));
        assert_eq!(pid_of_instance("luid_0x0_phys_0"), None);
    }

    #[test]
    fn browsers_and_the_shell_are_not_games() {
        assert!(not_a_game("chrome"));
        assert!(not_a_game("Explorer"));
        assert!(!not_a_game("eldenring"));
        assert!(!not_a_game("blender"));
    }

    /// `cargo test samples_this_pc -- --ignored --nocapture` prints real readings.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn samples_this_pc() {
        let mut s = win::Sampler::new();
        s.sample(false);
        std::thread::sleep(Duration::from_millis(1000));
        let m = s.sample(false);
        println!("{m:#?}");
        assert!(m.ram > 0.0 && m.disk_total_gb > 0.0);
    }
}
