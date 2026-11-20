// Jarvis Voice Engine: Web Audio API VAD (Voice Activity Detection) + MediaRecorder + Groq Whisper STT,
// plus natural pt-BR Neural TTS (Edge TTS).
//
// Avoids webkitSpeechRecognition completely (which fails in Edge WebView2 and caused 350ms restart loops).

import { Bridge } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";

export type VoiceState = "idle" | "wake" | "recording" | "transcribing";

export class JarvisVoice {
  private mediaRecorder: MediaRecorder | null = null;
  private mediaStream: MediaStream | null = null;
  private chunks: Blob[] = [];
  private autoStopTimer: number | null = null;
  private vadInterval: number | null = null;
  private audioCtx: AudioContext | null = null;
  private currentAudio: HTMLAudioElement | null = null;

  // Continuous wake-word VAD state (PCM ring buffer + adaptive noise floor)
  private wakeStream: MediaStream | null = null;
  private wakeCtx: AudioContext | null = null;
  private wakeProcessor: ScriptProcessorNode | null = null;
  private wakeZeroGain: GainNode | null = null;
  private wakeBusy = false;
  private wakeStarting = false;
  private pendingWakeBlob: Blob | null = null;

  state: VoiceState = "idle";
  onStateChange?: (state: VoiceState, detail?: string) => void;
  onCommand?: (text: string) => void;

  private setState(next: VoiceState, detail?: string) {
    if (this.state === next) return;
    const prev = this.state;
    this.state = next;
    // Never trigger UI redraws for internal idle <-> wake transitions
    if ((prev === "idle" && next === "wake") || (prev === "wake" && next === "idle")) {
      return;
    }
    this.onStateChange?.(next, detail);
  }

  /** Starts or stops the background VAD wake-word listener based on settings. */
  syncWakeWord() {
    const enabled = State.settings.jarvisWakeWordEnabled ?? true;
    if (
      enabled &&
      !this.wakeProcessor &&
      !this.wakeStarting &&
      this.state !== "recording" &&
      this.state !== "transcribing"
    ) {
      void this.startWakeVad();
    } else if (!enabled && (this.wakeProcessor || this.wakeStarting)) {
      this.stopWakeVad();
    }
  }

  private async startWakeVad() {
    if (this.wakeProcessor || this.wakeStarting || !navigator.mediaDevices?.getUserMedia) return;
    this.wakeStarting = true;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      // If stopWakeVad() was called while waiting for getUserMedia, abort cleanly
      if (!this.wakeStarting) {
        for (const t of stream.getTracks()) t.stop();
        return;
      }
      this.wakeStream = stream;
      const ctx = new AudioContext();
      this.wakeCtx = ctx;
      if (ctx.state === "suspended") {
        await ctx.resume().catch(() => {});
      }

      const source = ctx.createMediaStreamSource(stream);
      const bufferSize = 2048;
      const processor = ctx.createScriptProcessor(bufferSize, 1, 1);
      // Connect through a zero-gain node so ScriptProcessor fires without audio feedback
      const zeroGain = ctx.createGain();
      zeroGain.gain.value = 0;
      source.connect(processor);
      processor.connect(zeroGain);
      zeroGain.connect(ctx.destination);
      this.wakeProcessor = processor;
      this.wakeZeroGain = zeroGain;

      const sampleRate = ctx.sampleRate || 48000;
      const preRollMaxSamples = Math.round(sampleRate * 0.4); // 400ms pre-roll so "Ei" is never clipped
      const preRollChunks: Float32Array[] = [];
      let preRollSamples = 0;

      let speechChunks: Float32Array[] = [];
      let speechSamples = 0;
      let capturing = false;
      let speakingSince = 0;
      let silenceSince = 0;
      let noiseFloor = 0.004;

      const flushUtterance = () => {
        if (!capturing) return;
        capturing = false;
        silenceSince = 0;
        const dur = performance.now() - speakingSince;
        const collected = speechChunks;
        const totalSamples = speechSamples;
        speechChunks = [];
        speechSamples = 0;

        if (dur >= 180 && totalSamples > sampleRate * 0.25) {
          const wav = encodeWav16k(collected, totalSamples, sampleRate);
          void this.enqueueWakeBlob(wav);
        }
      };

      processor.onaudioprocess = (ev) => {
        if (this.state === "recording" || this.state === "transcribing") {
          capturing = false;
          speechChunks = [];
          speechSamples = 0;
          return;
        }
        // Don't listen to ourselves while TTS is playing
        if (this.currentAudio && !this.currentAudio.paused) {
          capturing = false;
          speechChunks = [];
          speechSamples = 0;
          return;
        }

        const input = ev.inputBuffer.getChannelData(0);
        let sum = 0;
        for (let i = 0; i < input.length; i++) {
          const s = input[i];
          sum += s * s;
        }
        const rms = Math.sqrt(sum / input.length);
        const now = performance.now();

        // Adaptive threshold: sensitive to quiet speech while tracking room noise floor
        const threshold = Math.max(0.007, Math.min(0.03, noiseFloor * 2.5 + 0.0035));
        const copy = new Float32Array(input);

        if (rms > threshold) {
          silenceSince = 0;
          if (!capturing) {
            capturing = true;
            speakingSince = now;
            speechChunks = [...preRollChunks, copy];
            speechSamples = preRollSamples + copy.length;
            preRollChunks.length = 0;
            preRollSamples = 0;
          } else {
            speechChunks.push(copy);
            speechSamples += copy.length;
            if (now - speakingSince > 6500) {
              flushUtterance();
            }
          }
        } else {
          // Update adaptive noise floor only during silence
          if (!capturing) {
            noiseFloor = noiseFloor * 0.95 + rms * 0.05;
            preRollChunks.push(copy);
            preRollSamples += copy.length;
            while (preRollSamples > preRollMaxSamples && preRollChunks.length > 1) {
              const dropped = preRollChunks.shift()!;
              preRollSamples -= dropped.length;
            }
          } else {
            speechChunks.push(copy);
            speechSamples += copy.length;
            if (silenceSince === 0) silenceSince = now;
            else if (now - silenceSince >= 480) {
              flushUtterance();
            }
          }
        }
      };
    } catch {
      this.stopWakeVad();
    } finally {
      this.wakeStarting = false;
    }
  }

  private stopWakeVad() {
    this.wakeStarting = false;
    this.pendingWakeBlob = null;
    if (this.wakeProcessor) {
      this.wakeProcessor.onaudioprocess = null;
      try {
        this.wakeProcessor.disconnect();
      } catch {
        // ignore
      }
      this.wakeProcessor = null;
    }
    if (this.wakeZeroGain) {
      try {
        this.wakeZeroGain.disconnect();
      } catch {
        // ignore
      }
      this.wakeZeroGain = null;
    }
    if (this.wakeCtx) {
      void this.wakeCtx.close().catch(() => {});
      this.wakeCtx = null;
    }
    if (this.wakeStream) {
      for (const t of this.wakeStream.getTracks()) t.stop();
      this.wakeStream = null;
    }
  }

  private async enqueueWakeBlob(blob: Blob) {
    if (this.wakeBusy) {
      // Keep the latest utterance so rapid retries are never silently dropped
      this.pendingWakeBlob = blob;
      return;
    }
    this.wakeBusy = true;
    try {
      let current: Blob | null = blob;
      while (current && this.state !== "recording" && this.state !== "transcribing") {
        const triggered = await this.checkWakeBlob(current);
        if (triggered) {
          this.pendingWakeBlob = null;
          break;
        }
        current = this.pendingWakeBlob;
        this.pendingWakeBlob = null;
      }
    } finally {
      this.wakeBusy = false;
    }
  }

  private async checkWakeBlob(blob: Blob): Promise<boolean> {
    try {
      const base64 = await blobToBase64(blob);
      const raw = (await Bridge.jarvisTranscribe(base64, "audio/wav")).trim();
      if (!raw) return false;

      const match = matchWakeWord(raw, State.settings.jarvisWakeWord);
      if (!match) return false;

      const after = raw.slice(match.endIndex).replace(/^[,.\s:!?-]+/, "").trim();
      if (after.length >= 2) {
        Sound.play("pop");
        this.onCommand?.(after);
      } else {
        // User just said "Ei, Jarvis" -> open active command recording!
        await this.startRecording();
      }
      return true;
    } catch {
      // Ignore silent/transient transcription errors during background wake check
      return false;
    }
  }

  /** Toggles push-to-talk recording (used by mic button or hotkey). */
  async toggleRecording() {
    if (this.state === "recording") {
      this.stopRecording();
      return;
    }
    if (this.state === "transcribing") return;
    await this.startRecording();
  }

  async startRecording() {
    this.stopWakeVad();
    if (this.currentAudio) {
      this.currentAudio.pause();
      this.currentAudio = null;
    }
    Sound.play("pop");

    if (!navigator.mediaDevices?.getUserMedia) return;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      this.mediaStream = stream;
      this.chunks = [];
      const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
        ? "audio/webm;codecs=opus"
        : MediaRecorder.isTypeSupported("audio/webm")
          ? "audio/webm"
          : "";
      const recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);
      this.mediaRecorder = recorder;

      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) this.chunks.push(e.data);
      };

      recorder.onstop = () => {
        this.cleanupRecording();
        const blobType = mimeType || recorder.mimeType || "audio/webm";
        const blob = new Blob(this.chunks, { type: blobType });
        this.chunks = [];
        void this.transcribeBlob(blob, blobType);
      };

      recorder.start();
      this.setState("recording", "Fala…");

      // Automatic silence detection (stops 1.2s after user finishes speaking)
      const ctx = new AudioContext();
      this.audioCtx = ctx;
      if (ctx.state === "suspended") {
        await ctx.resume().catch(() => {});
      }
      const source = ctx.createMediaStreamSource(stream);
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      source.connect(analyser);
      const data = new Uint8Array(analyser.fftSize);
      let heardSpeech = false;
      let silentSince = 0;

      this.vadInterval = window.setInterval(() => {
        analyser.getByteTimeDomainData(data);
        let sum = 0;
        for (let i = 0; i < data.length; i++) {
          const v = (data[i] - 128) / 128;
          sum += v * v;
        }
        const rms = Math.sqrt(sum / data.length);
        const now = performance.now();
        if (rms > 0.01) {
          heardSpeech = true;
          silentSince = 0;
        } else if (heardSpeech) {
          if (silentSince === 0) silentSince = now;
          else if (now - silentSince > 950) {
            this.stopRecording();
          }
        }
      }, 90);

      this.autoStopTimer = window.setTimeout(() => {
        if (this.state === "recording") this.stopRecording();
      }, 9000);
    } catch {
      this.setState("idle");
      this.syncWakeWord();
    }
  }

  stopRecording() {
    if (this.autoStopTimer !== null) {
      clearTimeout(this.autoStopTimer);
      this.autoStopTimer = null;
    }
    if (this.vadInterval !== null) {
      clearInterval(this.vadInterval);
      this.vadInterval = null;
    }
    if (this.mediaRecorder && this.mediaRecorder.state !== "inactive") {
      this.mediaRecorder.stop();
    } else {
      this.cleanupRecording();
      this.setState("idle");
      this.syncWakeWord();
    }
  }

  private cleanupRecording() {
    if (this.vadInterval !== null) {
      clearInterval(this.vadInterval);
      this.vadInterval = null;
    }
    if (this.audioCtx) {
      void this.audioCtx.close().catch(() => {});
      this.audioCtx = null;
    }
    if (this.mediaStream) {
      for (const t of this.mediaStream.getTracks()) t.stop();
      this.mediaStream = null;
    }
    this.mediaRecorder = null;
  }

  private async transcribeBlob(blob: Blob, mimeType: string) {
    if (blob.size < 600) {
      this.setState("idle");
      this.syncWakeWord();
      return;
    }
    this.setState("transcribing", "Transcrevendo…");
    try {
      const base64 = await blobToBase64(blob);
      const text = await Bridge.jarvisTranscribe(base64, mimeType);
      this.setState("idle");
      this.syncWakeWord();
      if (text.trim()) this.onCommand?.(text.trim());
    } catch (err) {
      this.setState("idle", String(err));
      this.syncWakeWord();
    }
  }

  /** Speaks a reply using natural pt-BR Neural TTS (Edge TTS), with Web Speech fallback. */
  async speak(text: string) {
    if (!(State.settings.jarvisTtsEnabled ?? true)) return;
    const clean = text.trim();
    if (!clean) return;

    if (this.currentAudio) {
      this.currentAudio.pause();
      this.currentAudio = null;
    }
    if ("speechSynthesis" in window) {
      window.speechSynthesis.cancel();
    }

    try {
      const b64 = await Bridge.jarvisTts(clean);
      if (b64) {
        const audio = new Audio(`data:audio/mp3;base64,${b64}`);
        this.currentAudio = audio;
        await audio.play();
        return;
      }
    } catch {
      // Fallback to browser SpeechSynthesis below
    }

    if (!("speechSynthesis" in window)) return;
    try {
      const utter = new SpeechSynthesisUtterance(clean);
      utter.lang = "pt-BR";
      utter.rate = 1.05;
      const voices = window.speechSynthesis.getVoices();
      const ptVoice =
        voices.find((v) => v.lang.toLowerCase().startsWith("pt-br") && v.name.toLowerCase().includes("natural")) ??
        voices.find((v) => v.lang.toLowerCase().startsWith("pt-br")) ??
        voices.find((v) => v.lang.toLowerCase().startsWith("pt"));
      if (ptVoice) utter.voice = ptVoice;
      window.speechSynthesis.speak(utter);
    } catch {
      // ignore TTS errors
    }
  }
}

const WAKE_VARIANTS = [
  "jarvis",
  "javes",
  "javis",
  "javiz",
  "jarves",
  "jarviz",
  "djarvis",
  "charvis",
  "gervis",
  "jarvi",
];

function stripAccentsWithMap(input: string): { clean: string; toOrig: number[] } {
  let clean = "";
  const toOrig: number[] = [];
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    const norm = ch.normalize("NFD").replace(/[\u0300-\u036f]/g, "");
    for (let j = 0; j < norm.length; j++) {
      clean += norm[j];
      toOrig.push(i);
    }
  }
  toOrig.push(input.length);
  return { clean, toOrig };
}

function matchWakeWord(raw: string, configuredWake?: string): { endIndex: number } | null {
  const { clean, toOrig } = stripAccentsWithMap(raw);
  const prefix = "(?:(?:ei|oi|hey|ok|fala|e\\s+ai)[\\s,.:!?-]+)?";
  const variantsGroup = `(?:${WAKE_VARIANTS.join("|")})`;
  const phoneticRegex = new RegExp(`\\b${prefix}${variantsGroup}\\b`, "i");

  const m = phoneticRegex.exec(clean);
  if (m) {
    return { endIndex: toOrig[m.index + m[0].length] ?? raw.length };
  }

  const custom = (configuredWake || "").trim();
  if (custom) {
    const { clean: customClean } = stripAccentsWithMap(custom.toLowerCase());
    const wakePattern = customClean
      .split(/[\s,.:!?-]+/)
      .filter(Boolean)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join("[\\s,.:!?-]+");
    if (wakePattern) {
      const customRegex = new RegExp(`\\b${wakePattern}\\b`, "i");
      const cm = customRegex.exec(clean);
      if (cm) {
        return { endIndex: toOrig[cm.index + cm[0].length] ?? raw.length };
      }
    }
  }

  return null;
}

function encodeWav16k(chunks: Float32Array[], totalSamples: number, inputSampleRate: number): Blob {
  const targetRate = 16000;
  const ratio = inputSampleRate / targetRate;
  const outLength = Math.max(1, Math.floor(totalSamples / ratio));
  const buffer = new ArrayBuffer(44 + outLength * 2);
  const view = new DataView(buffer);

  const writeStr = (offset: number, str: string) => {
    for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i));
  };

  writeStr(0, "RIFF");
  view.setUint32(4, 36 + outLength * 2, true);
  writeStr(8, "WAVE");
  writeStr(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // Mono
  view.setUint32(24, targetRate, true);
  view.setUint32(28, targetRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeStr(36, "data");
  view.setUint32(40, outLength * 2, true);

  // Merge chunks into a single view for fast linear resampling
  const merged = new Float32Array(totalSamples);
  let pos = 0;
  for (const c of chunks) {
    merged.set(c, pos);
    pos += c.length;
  }

  let offset = 44;
  for (let i = 0; i < outLength; i++) {
    const idx = i * ratio;
    const i0 = Math.floor(idx);
    const i1 = Math.min(totalSamples - 1, i0 + 1);
    const frac = idx - i0;
    const sample = merged[i0] * (1 - frac) + merged[i1] * frac;
    const clamped = Math.max(-1, Math.min(1, sample));
    view.setInt16(offset, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
    offset += 2;
  }

  return new Blob([buffer], { type: "audio/wav" });
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => {
      const res = typeof reader.result === "string" ? reader.result : "";
      const comma = res.indexOf(",");
      resolve(comma >= 0 ? res.slice(comma + 1) : res);
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

export const Voice = new JarvisVoice();
