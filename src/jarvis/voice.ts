// Jarvis Voice Engine: Web Audio API VAD (Voice Activity Detection) + MediaRecorder + Groq Whisper STT,
// plus natural pt-BR Neural TTS (Edge TTS).
//
// Avoids webkitSpeechRecognition completely (which fails in Edge WebView2 and caused 350ms restart loops).

import { Bridge } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";

/** "listening": "Jarvis" was heard alone — the next sentence is the command. */
export type VoiceState = "idle" | "wake" | "listening" | "recording" | "transcribing";

/** Which microphone, as getUserMedia constraints (Ajustes → IA & Voz). */
function micConstraints(): MediaTrackConstraints {
  const id = (State.settings.jarvisMicDevice ?? "").trim();
  const base: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };
  // "communications": the device Windows uses for calls (often the headset).
  return id ? { ...base, deviceId: { exact: id } } : base;
}

/** How loud a voice must be, per sensitivity: [floor, factor over the room noise, cap]. */
export function voiceThreshold(noiseFloor: number, sensitivity: string | undefined, strict = false): number {
  const [min, factor, cap] =
    sensitivity === "baixa" ? [0.01, 3, 0.035] : sensitivity === "media" ? [0.007, 2.5, 0.022] : [0.004, 2, 0.014];
  const t = Math.max(min, Math.min(cap, noiseFloor * factor + min / 2));
  // Media playing on the speakers: a bit stricter, so they do not wake it.
  return strict ? Math.min(cap * 1.6, t * 1.5) : t;
}

/** A line in %LOCALAPPDATA%\OpenJarvis\openjarvis.log, to see what the voice heard. */
function vlog(line: string) {
  void Bridge.jarvisLog(`voice: ${line}`).catch(() => {});
}

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
  private pendingWakeBlob: Utterance | null = null;
  /** Background checks paused until then (all transcription keys at their limit). */
  private pauseUntil = 0;
  /** Media is playing (YouTube, Spotify…): only a louder, closer voice counts. */
  mediaPlaying = false;
  /** A short message for the notch ("limite de transcrição"…). */
  onNotice?: (text: string) => void;
  /** Live microphone level and the current threshold (the meter in Ajustes). */
  onLevel?: (level: number, threshold: number) => void;

  /** Microphone or sensitivity changed: listen again with the new setup. */
  restart() {
    this.stopWakeVad();
    this.syncWakeWord();
  }

  /** The native side spots the enrolled "Jarvis" (wakeword.rs): this side then
   *  does not listen in the background at all. */
  localWake = false;

  setLocalWake(on: boolean) {
    if (this.localWake === on) return;
    this.localWake = on;
    vlog(on ? "reconhecimento local ligado (cadastro de voz)" : "reconhecimento na nuvem");
    if (on) {
      // The cloud listener stops — including a "listening" it had open.
      this.endListening();
      this.stopWakeVad();
    }
    this.syncWakeWord();
  }

  /** Events of the native wake word: state changes and the transcribed command. */
  attachLocal() {
    void Bridge.onVoiceLocalState((st) => {
      if (st === "listening") {
        Sound.play("pop");
        this.setState("listening", "Fala…");
      } else if (st === "transcribing") {
        this.setState("transcribing", "Transcrevendo…");
      } else if (this.state === "listening" || this.state === "transcribing") {
        this.setState("idle");
      }
    });
    void Bridge.onVoiceLocal(({ text, error }) => {
      if (this.state === "listening" || this.state === "transcribing") this.setState("idle");
      if (error) {
        vlog(`erro na transcrição (local): ${error.slice(0, 200)}`);
        if (/429|quota|limite|rate/i.test(error)) this.onNotice?.("Limite de transcrição atingido. Tente de novo em um minuto.");
        return;
      }
      const raw = text.trim();
      const m = matchWakeWord(raw, State.settings.jarvisWakeWord);
      const command = commandAfterWake(raw, m);
      if (!command || isNoise(command)) {
        vlog(`nada entendido depois de "Jarvis"${raw ? `: "${raw.slice(0, 80)}"` : ""}`);
        return;
      }
      vlog(`comando: "${command.slice(0, 120)}"`);
      this.onCommand?.(command);
    });
  }

  state: VoiceState = "idle";
  /** Until when (performance.now()) a sentence without "Jarvis" counts as the command. */
  private listenUntil = 0;
  /** Set when "Jarvis" was heard alone: the audio thread starts recording the command. */
  private listenArmed = false;
  private listenTimer: number | null = null;
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
    // The native wake word is listening: nothing to do here.
    const enabled = (State.settings.jarvisWakeWordEnabled ?? true) && !this.localWake;
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
      const stream = await navigator.mediaDevices
        .getUserMedia({ audio: micConstraints() })
        .catch((err) => {
          vlog(`microfone escolhido indisponível (${String(err).slice(0, 80)}); usando o padrão`);
          return navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
        });
      const label = stream.getAudioTracks()[0]?.label ?? "?";
      vlog(`ouvindo pelo microfone: ${label} (sensibilidade ${State.settings.jarvisMicSensitivity ?? "alta"})`);
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

      // Everything heard since the last sentence ended (up to 4 s): a command
      // said right after "Jarvis", while the name was still being checked.
      let tail: Float32Array[] = [];
      let tailSamples = 0;
      const tailMax = Math.round(sampleRate * 4);

      // The command after "Jarvis": one continuous recording, not sentences.
      let cmdChunks: Float32Array[] = [];
      let cmdSamples = 0;
      let cmdStart = 0;
      let cmdHeard = false;
      let cmdSilence = 0;
      /** The loudest the user's own voice got in this recording (smoothed). */
      let cmdPeak = 0;

      const finishCommand = (reason: string) => {
        const chunks = cmdChunks;
        const total = cmdSamples;
        cmdChunks = [];
        cmdSamples = 0;
        vlog(`gravação do comando: ${(total / sampleRate).toFixed(1)} s (${reason})`);
        if (!cmdHeard && reason === "tempo") {
          vlog("nenhum comando depois de \"Jarvis\"");
          this.endListening();
          return;
        }
        void this.finishListening(encodeWav16k(chunks, total, sampleRate));
      };

      const flushUtterance = () => {
        if (!capturing) return;
        tail = [];
        tailSamples = 0;
        capturing = false;
        silenceSince = 0;
        const dur = performance.now() - speakingSince;
        const collected = speechChunks;
        const totalSamples = speechSamples;
        speechChunks = [];
        speechSamples = 0;

        // Under 0.3 s it is a click or a cough ("Jarvis" alone takes ~0.4 s).
        if (dur >= 300 && totalSamples > sampleRate * 0.3) {
          if (performance.now() < this.pauseUntil) return;
          // "Jarvis" is at the start: only the first 2.5 s go out to be checked,
          // the whole sentence only once the wake word is there.
          const headSamples = Math.min(totalSamples, Math.round(sampleRate * 2.5));
          const full = encodeWav16k(collected, totalSamples, sampleRate);
          const head = headSamples < totalSamples ? encodeWav16k(collected, headSamples, sampleRate) : full;
          void this.enqueueWakeBlob({ head, full });
        }
      };

      processor.onaudioprocess = (ev) => {
        if (this.state === "recording" || this.state === "transcribing") {
          capturing = false;
          speechChunks = [];
          speechSamples = 0;
          return;
        }
        // Don't listen to ourselves while TTS is playing (neural voice or the Windows fallback)
        if ((this.currentAudio && !this.currentAudio.paused) || ("speechSynthesis" in window && window.speechSynthesis.speaking)) {
          capturing = false;
          speechChunks = [];
          speechSamples = 0;
          return;
        }

        const input = ev.inputBuffer.getChannelData(0);
        if (this.state === "listening" || this.listenArmed) {
          const frame = new Float32Array(input);
          let e = 0;
          for (let i = 0; i < frame.length; i++) e += frame[i] * frame[i];
          const level = Math.sqrt(e / frame.length);
          const t = performance.now();
          if (this.listenArmed) {
            // Start from what was said since "Jarvis" ended.
            this.listenArmed = false;
            cmdChunks = [...tail];
            cmdSamples = tailSamples;
            cmdStart = t;
            cmdHeard = false;
            cmdSilence = 0;
            cmdPeak = 0;
            capturing = false;
            speechChunks = [];
            speechSamples = 0;
          }
          cmdChunks.push(frame);
          cmdSamples += frame.length;
          // Relative to the room: a voice is clearly above the noise floor.
          const base = voiceThreshold(noiseFloor, State.settings.jarvisMicSensitivity);
          this.onLevel?.(level, base);
          // Once the user spoke, "silence" is well below *their* voice — so the
          // room talking behind them does not keep the recording open.
          if (level > base) cmdPeak = Math.max(cmdPeak * 0.995, level);
          const speaking = level > Math.max(base, cmdHeard ? cmdPeak * 0.3 : 0);
          if (speaking) {
            cmdHeard = true;
            cmdSilence = 0;
          } else if (cmdHeard) {
            if (cmdSilence === 0) cmdSilence = t;
            // A breath mid-sentence (up to ~1.8 s) does not end the command.
            else if (t - cmdSilence >= 1800) finishCommand("silêncio");
          }
          if (cmdSamples > 0 && t - cmdStart > (cmdHeard ? 20000 : 8000)) finishCommand(cmdHeard ? "máximo" : "tempo");
          return;
        }
        tail.push(new Float32Array(input));
        tailSamples += input.length;
        while (tailSamples > tailMax && tail.length > 1) tailSamples -= tail.shift()!.length;
        let sum = 0;
        for (let i = 0; i < input.length; i++) {
          const s = input[i];
          sum += s * s;
        }
        const rms = Math.sqrt(sum / input.length);
        const now = performance.now();

        // Adaptive threshold: sensitive to quiet speech while tracking room noise floor
        // With media playing, a moderately stricter threshold so the speakers do
        // not keep waking it (the command after "Jarvis" is recorded above).
        const threshold = voiceThreshold(noiseFloor, State.settings.jarvisMicSensitivity, this.mediaPlaying);
        this.onLevel?.(rms, threshold);
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
            // A short breath ("Jarvis… toca X") is not the end of the sentence.
            else if (now - silenceSince >= 800) {
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

  private async enqueueWakeBlob(blob: Utterance) {
    if (this.wakeBusy) {
      // Keep the latest utterance so rapid retries are never silently dropped
      this.pendingWakeBlob = blob;
      return;
    }
    this.wakeBusy = true;
    try {
      let current: Utterance | null = blob;
      while (current && this.state !== "recording" && this.state !== "transcribing") {
        await this.checkWakeBlob(current);
        // The next sentence (often the command after "Jarvis") is checked too.
        current = this.pendingWakeBlob;
        this.pendingWakeBlob = null;
      }
    } finally {
      this.wakeBusy = false;
    }
  }

  private async transcribeWav(blob: Blob): Promise<string | null> {
    try {
      const base64 = await blobToBase64(blob);
      return (await Bridge.jarvisTranscribe(base64, "audio/wav")).trim();
    } catch (err) {
      const msg = String(err);
      vlog(`erro na transcrição: ${msg.slice(0, 200)}`);
      if (/429|quota|limite|rate/i.test(msg)) {
        // Every key is at its limit: stop sending for a while, and say so once.
        this.pauseUntil = performance.now() + 30000;
        this.onNotice?.("Limite de transcrição atingido. Volto a ouvir em 30 s — ou use o botão do microfone depois.");
      }
      return null;
    }
  }

  private async checkWakeBlob(utt: Utterance): Promise<boolean> {
    const following = this.state === "listening" && performance.now() < this.listenUntil;
    // Waiting for the command: the whole sentence. Otherwise its first seconds.
    let raw = (await this.transcribeWav(following ? utt.full : utt.head)) ?? "";
    if (!following && raw && utt.head !== utt.full && matchWakeWord(raw, State.settings.jarvisWakeWord)) {
      raw = (await this.transcribeWav(utt.full)) ?? raw;
    }
    if (!raw || isNoise(raw)) {
      if (raw) vlog(`ignorado (ruído): "${raw.slice(0, 60)}"`);
      return false;
    }

    const match = matchWakeWord(raw, State.settings.jarvisWakeWord);
    if (!match && !following) {
      vlog(`sem "Jarvis": "${raw.slice(0, 80)}"`);
      return false;
    }
    const command = commandAfterWake(raw, match);
    if (command.length >= 2) {
      vlog(`comando: "${command.slice(0, 120)}"`);
      this.endListening();
      Sound.play("pop");
      this.onCommand?.(command);
    } else {
      // Just "Jarvis": the next sentence (within 7 s) is the command — the
      // microphone stays open, so nothing said right after is lost.
      vlog(`"Jarvis" — ouvindo o comando`);
      Sound.play("pop");
      this.beginListening();
    }
    return true;
  }

  private beginListening() {
    vlog(`ouvindo o comando (mídia tocando: ${this.mediaPlaying ? "sim" : "não"})`);
    // Safety net only: the audio thread ends the recording (silence / 8–20 s).
    this.armListening(22000);
    this.listenArmed = true;
    this.setState("listening", "Fala…");
  }

  /** Transcribes the recording made after "Jarvis" and runs it. */
  private async finishListening(wav: Blob) {
    this.listenUntil = 0;
    if (this.listenTimer !== null) {
      clearTimeout(this.listenTimer);
      this.listenTimer = null;
    }
    this.setState("transcribing", "Transcrevendo…");
    const raw = (await this.transcribeWav(wav)) ?? "";
    this.setState("idle");
    const m = matchWakeWord(raw, State.settings.jarvisWakeWord);
    const command = commandAfterWake(raw, m);
    if (!command || isNoise(command)) {
      vlog(`nada entendido depois de "Jarvis"${raw ? `: "${raw.slice(0, 80)}"` : ""}`);
      return;
    }
    vlog(`comando: "${command.slice(0, 120)}"`);
    Sound.play("pop");
    this.onCommand?.(command);
  }

  private armListening(ms: number) {
    this.listenUntil = performance.now() + ms;
    if (this.listenTimer !== null) clearTimeout(this.listenTimer);
    this.listenTimer = window.setTimeout(() => {
      if (this.state === "listening") {
        vlog("nenhum comando depois de \"Jarvis\"");
        this.endListening();
      }
    }, ms + 300);
  }

  private endListening() {
    this.listenUntil = 0;
    this.listenArmed = false;
    if (this.listenTimer !== null) {
      clearTimeout(this.listenTimer);
      this.listenTimer = null;
    }
    if (this.state === "listening") this.setState("idle");
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
    this.endListening();
    this.stopWakeVad();
    if (this.currentAudio) {
      this.currentAudio.pause();
      this.currentAudio = null;
    }
    Sound.play("pop");

    if (!navigator.mediaDevices?.getUserMedia) return;

    try {
      const stream = await navigator.mediaDevices
        .getUserMedia({ audio: micConstraints() })
        .catch(() => navigator.mediaDevices.getUserMedia({ audio: true }));
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
        const limit = voiceThreshold(0.003, State.settings.jarvisMicSensitivity);
        this.onLevel?.(rms, limit);
        if (rms > limit) {
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
      const text = (await Bridge.jarvisTranscribe(base64, mimeType)).trim();
      this.setState("idle");
      this.syncWakeWord();
      if (text && !isNoise(text)) {
        vlog(`comando (microfone): "${text.slice(0, 120)}"`);
        this.onCommand?.(text);
      } else {
        vlog(`nada entendido (microfone)${text ? `: "${text.slice(0, 60)}"` : ""}`);
      }
    } catch (err) {
      vlog(`erro na transcrição (microfone): ${String(err).slice(0, 200)}`);
      this.setState("idle", String(err));
      this.syncWakeWord();
    }
  }

  /** Speaks a reply using natural pt-BR Neural TTS (Edge TTS), with Web Speech
   *  fallback. Resolves when it has finished speaking (or was cut off). */
  async speak(text: string) {
    if (!(State.settings.jarvisTtsEnabled ?? true)) return;
    // Said, not read: no "asterisco", no markdown, no emoji names.
    const clean = speakable(text);
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
        const done = new Promise<void>((resolve) => {
          audio.addEventListener("ended", () => resolve(), { once: true });
          // Cut off by a newer reply, or the clip failed: not speaking anymore.
          audio.addEventListener("pause", () => resolve(), { once: true });
          audio.addEventListener("error", () => resolve(), { once: true });
        });
        await audio.play();
        await done;
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
      const done = new Promise<void>((resolve) => {
        utter.onend = () => resolve();
        utter.onerror = () => resolve();
      });
      window.speechSynthesis.speak(utter);
      await done;
    } catch {
      // ignore TTS errors
    }
  }
}

/** One sentence heard in the background: its first seconds and the whole of it. */
interface Utterance {
  head: Blob;
  full: Blob;
}

/** The reply as it should be heard: the chat shows markdown, the voice must
 *  not read it ("asterisco", "cerquilha", a whole URL, emoji names). */
export function speakable(text: string): string {
  return (
    text
      // Code blocks are for the screen.
      .replace(/```[\s\S]*?```/g, "\n")
      // [texto](url) → texto; bare links → "o link".
      .replace(/\[([^\]]+)\]\((?:[^)]+)\)/g, "$1")
      .replace(/\bhttps?:\/\/\S+/gi, "o link")
      // Windows paths: "D:\Projetos\jarvis" → "Projetos jarvis".
      .replace(/\b[A-Za-z]:\\/g, "")
      .replace(/\\/g, " ")
      // Markdown marks: headings, quotes, bullets, emphasis, inline code.
      .replace(/^\s{0,3}#{1,6}\s+/gm, "")
      .replace(/^\s*>\s?/gm, "")
      .replace(/^\s*[-*+•]\s+/gm, "")
      .replace(/(\*\*|__|~~|`)/g, "")
      .replace(/(^|[\s(])[*_]([^*_\n]+)[*_](?=[\s).,!?:;]|$)/g, "$1$2")
      .replace(/[*#|]/g, " ")
      // Emoji and pictographs.
      .replace(/(\p{Extended_Pictographic}|\uFE0F|\u200D)/gu, "")
      // A line break is a pause.
      .replace(/([^.!?:;,\s])\s*\n+\s*/g, "$1. ")
      .replace(/\s*\n+\s*/g, " ")
      .replace(/\s{2,}/g, " ")
      .trim()
  );
}

/** Does the reply end by asking the user something? Then the answer can come
 *  without saying "Jarvis" again. A question followed by a short invitation
 *  ("…qual projeto? É só mandar.") counts too. */
export function asksBack(text: string): boolean {
  const tidy = (s: string) =>
    s
      .trim()
      // Trailing emoji, quotes, markdown and brackets do not count.
      .replace(/[\s"'”’)\]*_~`]+$/u, "")
      .replace(/(\p{Extended_Pictographic}|\uFE0F|\u200D|\s)+$/u, "")
      .trim();
  const t = tidy(text);
  if (t.endsWith("?")) return true;
  const q = t.lastIndexOf("?");
  if (q < 0) return false;
  const tail = t.slice(q + 1).trim();
  return tail.length <= 40 && /^(é só|e so|me (diga|diz|fala|avisa|conta)|pode (falar|mandar|dizer)|fala|manda|diz|estou ouvindo|tô ouvindo|to ouvindo)(?=[\s.!,]|$)/i.test(tail);
}

/** The command in a transcription: what follows "Jarvis" (and a second
 *  "Jarvis" right after it, said again out of habit). */
export function commandAfterWake(raw: string, match: { endIndex: number } | null): string {
  const clean = (s: string) => s.replace(/^[,.\s:!?-]+/, "").trim();
  let rest = clean(match ? raw.slice(match.endIndex) : raw);
  for (let i = 0; i < 2; i++) {
    const again = matchWakeWord(rest, State.settings.jarvisWakeWord);
    // Only when the name opens what is left ("Jarvis. Me faça…").
    if (!again || rest.slice(0, again.endIndex).trim().split(/\s+/).length > 2) break;
    rest = clean(rest.slice(again.endIndex));
  }
  return rest;
}

/** What Whisper "hears" in silence, music or noise — never a command. */
export function isNoise(text: string): boolean {
  const t = text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9 ]+/g, " ")
    .trim();
  if (t.length < 2) return true;
  const junk = [
    "obrigado", "obrigada", "obrigado por assistir", "obrigada por assistir", "legendas pela comunidade amara org",
    "legenda adriana zanotto", "tchau", "e ai", "hum", "uhum", "ah", "oh", "musica", "som", "aplausos", "risos",
    "inscreva se no canal", "thank you", "thanks for watching", "you",
  ];
  return junk.includes(t) || /^(legendas?|subtitles?) /.test(t) || /amara org/.test(t);
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
  "jarbas",
  "jarvys",
  "jervis",
  "garvis",
  "harvis",
  "yarvis",
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

export function matchWakeWord(raw: string, configuredWake?: string): { endIndex: number } | null {
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
