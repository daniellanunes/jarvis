//! Local wake word: the user's own "Jarvis", spotted on the PC.
//!
//! The microphone is read natively (cpal). Rustpotter compares every 30 ms of
//! audio with a model built from a few recordings of the user saying "Jarvis"
//! (the voice enrollment in Ajustes). Nothing leaves the PC until the name is
//! heard; then the command that follows is recorded on the same stream — no
//! gap, no second microphone — until the user's voice stops, and only that is
//! transcribed. The island gets:
//!
//! - `voice-local-state` "listening" | "transcribing" | "idle"
//! - `voice-local` { text, error }
//!
//! Without an enrollment this module stays off and the cloud wake word of
//! voice.ts keeps working as before.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use rustpotter::{Rustpotter, RustpotterConfig, WakewordLoad, WakewordRef, WakewordRefBuildFromBuffers, WakewordSave};

const RATE: usize = 16_000;
const SAMPLES_WANTED: usize = 6;

/// Requests for the audio thread.
enum Ctl {
    /// Settings or model changed: reopen the microphone and reload.
    Restart,
    /// Record one enrollment sample; the reply carries the WAV bytes.
    RecordSample(Sender<Result<Vec<u8>, String>>),
    /// Push to talk: record a command now, without the wake word.
    ListenNow,
}

static CONTROL: LazyLock<Mutex<Option<Sender<Ctl>>>> = LazyLock::new(|| Mutex::new(None));
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

fn dir() -> PathBuf {
    crate::settings::local_dir().join("wakeword")
}
fn model_path() -> PathBuf {
    dir().join("jarvis.rpw")
}
fn samples_dir() -> PathBuf {
    dir().join("samples")
}

fn sample_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(samples_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "wav")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

// ── Pure helpers (tested) ────────────────────────────────────────────────────

/// Linear resampling of mono audio to 16 kHz.
pub fn resample(input: &[f32], from_rate: u32) -> Vec<f32> {
    if from_rate as usize == RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / RATE as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let i0 = pos.floor() as usize;
            let i1 = (i0 + 1).min(input.len() - 1);
            let f = (pos - i0 as f64) as f32;
            input[i0] * (1.0 - f) + input[i1] * f
        })
        .collect()
}

pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// 16-bit mono 16 kHz WAV.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(RATE as u32).to_le_bytes());
    b.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// The voiced part of a recording (plus a little margin), by 30 ms energy.
pub fn trim_voice(samples: &[f32], threshold: f32) -> Option<&[f32]> {
    let frame = RATE * 30 / 1000;
    let voiced: Vec<usize> = samples.chunks(frame).enumerate().filter(|(_, c)| rms(c) > threshold).map(|(i, _)| i).collect();
    let (first, last) = (*voiced.first()?, *voiced.last()?);
    let margin = 4; // 120 ms
    let start = first.saturating_sub(margin) * frame;
    let end = ((last + 1 + margin) * frame).min(samples.len());
    Some(&samples[start..end])
}

/// When the user stopped talking: tracks their voice level and the room's.
pub struct EndOfSpeech {
    floor: f32,
    peak: f32,
    heard: bool,
    quiet_ms: f32,
    elapsed_ms: f32,
}

pub enum Speech {
    Going,
    /// The user spoke and then went quiet.
    Done,
    /// Nobody spoke in time.
    Nothing,
    /// Too long: cut here.
    Limit,
}

impl EndOfSpeech {
    pub fn new(room_floor: f32) -> Self {
        Self { floor: room_floor.max(0.002), peak: 0.0, heard: false, quiet_ms: 0.0, elapsed_ms: 0.0 }
    }

    /// Feed `ms` of audio at level `level`.
    pub fn feed(&mut self, level: f32, ms: f32) -> Speech {
        self.elapsed_ms += ms;
        let base = (self.floor * 2.2 + 0.003).max(0.006);
        if level > base {
            self.peak = (self.peak * 0.995).max(level);
        }
        // After the user spoke, "quiet" is well below *their* voice, so the room
        // talking behind them does not keep the recording open.
        let speaking = level > base.max(if self.heard { self.peak * 0.3 } else { 0.0 });
        if speaking {
            self.heard = true;
            self.quiet_ms = 0.0;
        } else if self.heard {
            self.quiet_ms += ms;
            if self.quiet_ms >= 700.0 {
                return Speech::Done;
            }
        }
        if !self.heard && self.elapsed_ms > 6000.0 {
            return Speech::Nothing;
        }
        if self.elapsed_ms > 12000.0 {
            return Speech::Limit;
        }
        Speech::Going
    }
}

// ── Settings → detector ──────────────────────────────────────────────────────

fn detector_config(threshold: f32) -> RustpotterConfig {
    let mut c = RustpotterConfig::default(); // 16 kHz mono f32 in
    c.detector.threshold = threshold.clamp(0.3, 0.8);
    c.detector.avg_threshold = (threshold * 0.4).clamp(0.12, 0.35);
    c.detector.min_scores = 4;
    c.filters.gain_normalizer.enabled = true;
    c
}

fn load_detector(threshold: f32) -> Result<Rustpotter, String> {
    let mut d = Rustpotter::new(&detector_config(threshold))?;
    let wake = WakewordRef::load_from_file(&model_path().to_string_lossy())?;
    d.add_wakeword_ref("jarvis", wake)?;
    Ok(d)
}

/// Local detection is on when there is an enrollment and the setting allows it.
fn wanted() -> bool {
    let s = crate::settings::load();
    s.jarvis_wake_word_enabled && s.jarvis_wake_engine != "cloud" && model_path().exists()
}

// ── Microphone ───────────────────────────────────────────────────────────────

/// The microphone chosen in Ajustes, by name ("" = Windows default for apps,
/// "communications" = the default for calls).
fn pick_device(host: &cpal::Host) -> Option<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let s = crate::settings::load();
    let wanted_name = if s.jarvis_mic_device == "communications" {
        crate::audio::state().ok().map(|a| a.mic_name).unwrap_or_default()
    } else {
        s.jarvis_mic_name.clone()
    };
    if !wanted_name.trim().is_empty() {
        let w = wanted_name.to_lowercase();
        if let Ok(devs) = host.input_devices() {
            for d in devs {
                if let Ok(n) = d.name() {
                    let n = n.to_lowercase();
                    if !n.is_empty() && (w.starts_with(&n) || n.starts_with(&w) || w.contains(&n)) {
                        return Some(d);
                    }
                }
            }
        }
    }
    host.default_input_device()
}

/// Opens the microphone; mono f32 blocks at the device rate go to the channel.
fn open_mic(tx: Sender<Vec<f32>>) -> Result<(cpal::Stream, u32, String), String> {
    use cpal::traits::{DeviceTrait, StreamTrait};
    let host = cpal::default_host();
    let device = pick_device(&host).ok_or("Nenhum microfone encontrado.")?;
    let name = device.name().unwrap_or_default();
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.clone().into();
    let err = |e| crate::log::line(format!("wakeword: microfone: {e}"));
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let _ = tx.send(data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect());
            },
            err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let _ = tx.send(data.chunks(channels).map(|f| f.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / channels as f32).collect());
            },
            err,
            None,
        ),
        other => return Err(format!("Formato de áudio não suportado: {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate, name))
}

// ── The audio thread ─────────────────────────────────────────────────────────

enum Mode {
    /// Spotting "Jarvis".
    Idle,
    /// Recording the command after it.
    Command { audio: Vec<f32>, end: EndOfSpeech },
    /// Recording one enrollment sample.
    Sample { audio: Vec<f32>, end: EndOfSpeech, reply: Sender<Result<Vec<u8>, String>> },
}

fn emit_state(app: &AppHandle, state: &str) {
    let _ = app.emit_to(crate::island::WINDOW_LABEL, "voice-local-state", state.to_string());
}

#[derive(Serialize, Clone)]
struct VoiceLocal {
    text: String,
    error: String,
}

fn transcribe_and_emit(app: &AppHandle, audio: Vec<f32>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        emit_state(&app, "transcribing");
        let wav = wav_bytes(&audio);
        let b64 = base64(&wav);
        let res = crate::jarvis_brain::jarvis_transcribe(b64, "audio/wav".into()).await;
        let msg = match res {
            Ok(text) => VoiceLocal { text, error: String::new() },
            Err(e) => VoiceLocal { text: String::new(), error: e },
        };
        let _ = app.emit_to(crate::island::WINDOW_LABEL, "voice-local", msg);
        emit_state(&app, "idle");
    });
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// One run with the microphone open. Returns when asked to restart.
fn run_session(app: &AppHandle, ctl: &Receiver<Ctl>) -> Result<(), String> {
    let (tx, audio_rx) = mpsc::channel::<Vec<f32>>();
    let (_stream, rate, mic) = open_mic(tx)?;
    let threshold = crate::settings::load().jarvis_wake_threshold;
    let mut detector = if model_path().exists() { Some(load_detector(threshold)?) } else { None };
    let detect = detector.is_some() && wanted();
    ACTIVE.store(detect, Ordering::Relaxed);
    crate::log::line(format!("wakeword: microfone {mic} ({rate} Hz), detecção local {}", if detect { "ligada" } else { "desligada" }));

    let frame = detector.as_ref().map(|d| d.get_samples_per_frame()).unwrap_or(RATE * 30 / 1000);
    let mut pending: Vec<f32> = Vec::new();
    let mut floor = 0.004f32;
    let mut mode = Mode::Idle;
    let mut cooldown_until = Instant::now();

    loop {
        // Requests first.
        while let Ok(c) = ctl.try_recv() {
            match c {
                Ctl::Restart => return Ok(()),
                Ctl::RecordSample(reply) => {
                    mode = Mode::Sample { audio: Vec::new(), end: EndOfSpeech::new(floor), reply };
                }
                Ctl::ListenNow => {
                    emit_state(app, "listening");
                    mode = Mode::Command { audio: Vec::new(), end: EndOfSpeech::new(floor) };
                }
            }
        }
        let block = match audio_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(b) => b,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("O microfone parou.".into()),
        };
        pending.extend(resample(&block, rate));

        while pending.len() >= frame {
            let chunk: Vec<f32> = pending.drain(..frame).collect();
            let level = rms(&chunk);
            let ms = frame as f32 * 1000.0 / RATE as f32;
            if matches!(mode, Mode::Idle) {
                // The room's level, from what is not a detection.
                floor = floor * 0.97 + level.min(0.05) * 0.03;
                if !detect || Instant::now() < cooldown_until {
                    continue;
                }
                if let Some(d) = detector.as_mut() {
                    if let Some(hit) = d.process_samples(chunk) {
                        crate::log::line(format!("voice: \"Jarvis\" local (score {:.2}, média {:.2})", hit.score, hit.avg_score));
                        emit_state(app, "listening");
                        mode = Mode::Command { audio: Vec::new(), end: EndOfSpeech::new(floor) };
                    }
                }
                continue;
            }
            if step(app, &mut mode, &chunk, level, ms, &mut floor) == Some(true) {
                cooldown_until = Instant::now() + Duration::from_millis(1000);
                if let Some(d) = detector.as_mut() {
                    d.reset();
                }
            }
        }
    }
}

/// Starts the audio thread (once). It reopens the microphone on Restart and
/// retries after errors.
pub fn spawn(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Ctl>();
    *CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    std::thread::spawn(move || loop {
        // The microphone is held only while it is used: local detection, or
        // an enrollment / push-to-talk request.
        if !wanted() {
            ACTIVE.store(false, Ordering::Relaxed);
            match rx.recv() {
                Ok(Ctl::Restart) => continue,
                Ok(first) => {
                    if let Err(e) = run_on_demand(&app, first, &rx) {
                        *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = e.clone();
                        crate::log::line(format!("wakeword: {e}"));
                    }
                    continue;
                }
                Err(_) => return,
            }
        }
        if let Err(e) = run_session(&app, &rx) {
            ACTIVE.store(false, Ordering::Relaxed);
            *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = e.clone();
            crate::log::line(format!("wakeword: {e} — nova tentativa em 5 s"));
            std::thread::sleep(Duration::from_secs(5));
        } else {
            LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
    });
}

/// With detection off: open the microphone for a request (enrollment sample,
/// push to talk) and the ones that follow, then close it after 20 s idle.
fn run_on_demand(app: &AppHandle, first: Ctl, ctl: &Receiver<Ctl>) -> Result<(), String> {
    let (atx, audio_rx) = mpsc::channel::<Vec<f32>>();
    let (_stream, rate, _mic) = open_mic(atx)?;
    let frame = RATE * 30 / 1000;
    let mut pending = Vec::new();
    // Room level unknown yet: a quiet default, adapted while idle.
    let mut floor = 0.004f32;
    let mut mode = Mode::Idle;
    let mut last_use = Instant::now();
    let mut next = Some(first);
    loop {
        let mut req = next.take();
        if req.is_none() {
            req = ctl.try_recv().ok();
        }
        if let Some(c) = req {
            last_use = Instant::now();
            match c {
                Ctl::Restart => return Ok(()),
                Ctl::RecordSample(reply) => mode = Mode::Sample { audio: Vec::new(), end: EndOfSpeech::new(floor), reply },
                Ctl::ListenNow => {
                    emit_state(app, "listening");
                    mode = Mode::Command { audio: Vec::new(), end: EndOfSpeech::new(floor) };
                }
            }
            continue;
        }
        if matches!(mode, Mode::Idle) && last_use.elapsed() > Duration::from_secs(20) {
            return Ok(());
        }
        let block = match audio_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(b) => b,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Err("O microfone parou.".into()),
        };
        pending.extend(resample(&block, rate));
        while pending.len() >= frame {
            let chunk: Vec<f32> = pending.drain(..frame).collect();
            let level = rms(&chunk);
            if let Some(done) = step(app, &mut mode, &chunk, level, 30.0, &mut floor) {
                if done {
                    last_use = Instant::now();
                }
            }
        }
    }
}

/// One frame for the Command / Sample modes (shared by both sessions).
/// Returns Some(true) when a recording just finished.
fn step(app: &AppHandle, mode: &mut Mode, chunk: &[f32], level: f32, ms: f32, floor: &mut f32) -> Option<bool> {
    match mode {
        Mode::Idle => {
            *floor = *floor * 0.97 + level.min(0.05) * 0.03;
            None
        }
        Mode::Command { audio, end } => {
            audio.extend_from_slice(chunk);
            match end.feed(level, ms) {
                Speech::Going => None,
                Speech::Nothing => {
                    crate::log::line("voice: nenhum comando depois de \"Jarvis\" (local)");
                    emit_state(app, "idle");
                    *mode = Mode::Idle;
                    Some(true)
                }
                Speech::Done | Speech::Limit => {
                    let recorded = std::mem::take(audio);
                    crate::log::line(format!("voice: comando gravado localmente: {:.1} s", recorded.len() as f32 / RATE as f32));
                    transcribe_and_emit(app, recorded);
                    *mode = Mode::Idle;
                    Some(true)
                }
            }
        }
        Mode::Sample { audio, end, .. } => {
            audio.extend_from_slice(chunk);
            let state = end.feed(level, ms);
            if matches!(state, Speech::Going) {
                return None;
            }
            let Mode::Sample { audio, reply, .. } = std::mem::replace(mode, Mode::Idle) else { unreachable!() };
            let res = match state {
                Speech::Nothing => Err("Não ouvi nada. Fale \"Jarvis\" um pouco mais alto.".to_string()),
                _ => match trim_voice(&audio, (*floor * 2.2 + 0.003).max(0.006)) {
                    Some(v) if v.len() >= RATE * 3 / 10 && v.len() <= RATE * 2 => Ok(wav_bytes(v)),
                    Some(v) if v.len() > RATE * 2 => Err("Ficou longo demais: diga só \"Jarvis\".".to_string()),
                    _ => Err("Muito curto. Diga \"Jarvis\" normalmente.".to_string()),
                },
            };
            let _ = reply.send(res);
            Some(true)
        }
    }
}

fn send(c: Ctl) -> Result<(), String> {
    CONTROL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .ok_or("O áudio local não iniciou.")?
        .send(c)
        .map_err(|_| "O áudio local parou.".to_string())
}

/// Settings changed (mic, sensitivity, engine): reopen and reload.
pub fn restart() {
    let _ = send(Ctl::Restart);
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeStatus {
    /// Local detection running now.
    pub active: bool,
    /// A voice model exists.
    pub enrolled: bool,
    /// Samples recorded so far (for the enrollment screen).
    pub samples: usize,
    pub samples_wanted: usize,
    pub error: String,
}

#[tauri::command]
pub fn wakeword_status() -> WakeStatus {
    WakeStatus {
        active: ACTIVE.load(Ordering::Relaxed),
        enrolled: model_path().exists(),
        samples: sample_files().len(),
        samples_wanted: SAMPLES_WANTED,
        error: LAST_ERROR.lock().map(|e| e.clone()).unwrap_or_default(),
    }
}

/// Records one "Jarvis" for the enrollment (waits until the user stops).
#[tauri::command]
pub async fn wakeword_record_sample(index: usize) -> Result<usize, String> {
    let (tx, rx) = mpsc::channel();
    send(Ctl::RecordSample(tx))?;
    let wav = tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Tempo esgotado: o microfone não respondeu.".to_string())??;
    std::fs::create_dir_all(samples_dir()).map_err(|e| e.to_string())?;
    std::fs::write(samples_dir().join(format!("{index:02}.wav")), &wav).map_err(|e| e.to_string())?;
    Ok(sample_files().len())
}

/// Builds the voice model from the recorded samples and turns detection on.
#[tauri::command]
pub fn wakeword_build() -> Result<WakeStatus, String> {
    let files = sample_files();
    if files.len() < 3 {
        return Err("Grave pelo menos 3 amostras.".into());
    }
    let mut buffers = std::collections::HashMap::new();
    for f in &files {
        buffers.insert(f.file_name().unwrap_or_default().to_string_lossy().to_string(), std::fs::read(f).map_err(|e| e.to_string())?);
    }
    let wake = WakewordRef::new_from_sample_buffers("jarvis".into(), None, None, buffers, 16).map_err(|e| format!("Não consegui criar o modelo: {e}"))?;
    wake.save_to_file(&model_path().to_string_lossy())?;
    crate::log::line(format!("wakeword: modelo criado com {} amostras", files.len()));
    restart();
    std::thread::sleep(Duration::from_millis(400));
    Ok(wakeword_status())
}

/// Removes the enrollment: back to the cloud wake word.
#[tauri::command]
pub fn wakeword_reset() -> WakeStatus {
    let _ = std::fs::remove_dir_all(dir());
    restart();
    std::thread::sleep(Duration::from_millis(200));
    wakeword_status()
}

/// Push to talk through the native microphone (no wake word).
#[tauri::command]
pub fn wakeword_listen_now() -> Result<(), String> {
    send(Ctl::ListenNow)
}

#[tauri::command]
pub fn wakeword_restart() {
    restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_duration() {
        let one_second_48k = vec![0.1f32; 48_000];
        assert_eq!(resample(&one_second_48k, 48_000).len(), 16_000);
        assert_eq!(resample(&vec![0.0; 16_000], 16_000).len(), 16_000);
    }

    #[test]
    fn wav_has_a_valid_header() {
        let w = wav_bytes(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + 6);
    }

    #[test]
    fn trimming_keeps_the_voice_and_a_margin() {
        let mut s = vec![0.0f32; 16_000];
        for v in &mut s[8_000..12_000] {
            *v = 0.2;
        }
        let t = trim_voice(&s, 0.01).unwrap();
        // 0.25 s of voice + 120 ms of margin on each side.
        assert!(t.len() >= 7_500 && t.len() <= 8_700, "{}", t.len());
        assert!(trim_voice(&vec![0.0; 16_000], 0.01).is_none());
    }

    #[test]
    fn speech_ends_when_the_user_goes_quiet_even_with_room_noise() {
        let mut e = EndOfSpeech::new(0.01);
        // Room noise at 0.012 before speaking: still waiting.
        for _ in 0..10 {
            assert!(matches!(e.feed(0.012, 30.0), Speech::Going));
        }
        // The user speaks loud and close.
        for _ in 0..30 {
            assert!(matches!(e.feed(0.12, 30.0), Speech::Going));
        }
        // Back to the room's chatter (above the floor, far below the voice): done in ~0.7 s.
        let mut done_after = 0;
        for i in 0..60 {
            if matches!(e.feed(0.03, 30.0), Speech::Done) {
                done_after = i;
                break;
            }
        }
        assert!((21..=26).contains(&done_after), "{done_after}");
    }

    #[test]
    fn silence_without_speech_gives_up() {
        let mut e = EndOfSpeech::new(0.004);
        let mut out = Speech::Going;
        for _ in 0..300 {
            out = e.feed(0.002, 30.0);
            if !matches!(out, Speech::Going) {
                break;
            }
        }
        assert!(matches!(out, Speech::Nothing));
    }
}

#[cfg(test)]
mod pipeline_probe {
    use super::*;

    fn read_wav(path: &std::path::Path) -> Vec<f32> {
        let b = std::fs::read(path).unwrap();
        b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()
    }

    fn detections(d: &mut Rustpotter, audio: &[f32]) -> Vec<f32> {
        let frame = d.get_samples_per_frame();
        let mut hits = Vec::new();
        // A second of silence around, as in a real stream.
        let mut a = vec![0.0f32; RATE];
        a.extend_from_slice(audio);
        a.extend(vec![0.0f32; RATE]);
        for chunk in a.chunks(frame) {
            if chunk.len() == frame {
                if let Some(h) = d.process_samples(chunk.to_vec()) {
                    hits.push(h.score);
                }
            }
        }
        hits
    }

    /// Enrollment from synthesized "Jarvis" samples, then a sentence with the
    /// name and one without (`cargo test wake_pipeline -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn wake_pipeline_detects_the_name_only() {
        let dir = std::env::temp_dir().join("jarvis-wake-test");
        let mut buffers = std::collections::HashMap::new();
        for i in 0..6 {
            let audio = read_wav(&dir.join(format!("sample{i}.wav")));
            let trimmed = trim_voice(&audio, 0.01).unwrap().to_vec();
            buffers.insert(format!("{i}"), wav_bytes(&trimmed));
        }
        let wake = WakewordRef::new_from_sample_buffers("jarvis".into(), None, None, buffers, 16).unwrap();
        let mut d = Rustpotter::new(&detector_config(0.5)).unwrap();
        d.add_wakeword_ref("jarvis", wake).unwrap();
        let pos = detections(&mut d, &read_wav(&dir.join("positive.wav")));
        d.reset();
        let neg = detections(&mut d, &read_wav(&dir.join("negative.wav")));
        println!("com \"Jarvis\": {pos:?}\nsem \"Jarvis\": {neg:?}");
        assert!(!pos.is_empty(), "não detectou o nome");
        assert!(neg.is_empty(), "detectou sem o nome");
    }
}
