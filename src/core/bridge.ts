// Thin wrapper over the Tauri commands/events. Every call is a no-op when the
// page is opened in a plain browser, so the island can be iterated on with
// `npm run dev` alone.

import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Settings } from "./state";
import type { RecapHistory, RecapPrefs } from "../recap/summary";
import type { SpotifyState } from "./spotify";

export const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!IS_TAURI) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[openjarvis] ${cmd} failed`, err);
    return null;
  }
}

export interface BootInfo {
  settings: Settings;
  /** Logical screen rect of the monitor the island lives on. */
  screen: { x: number; y: number; width: number; height: number; scale: number };
  version: string;
  hookPath: string;
  /** False where the OS has no global cursor (Wayland): see Island.followPageCursor. */
  cursorPoll: boolean;
}

export const Bridge = {
  boot: () => call<BootInfo>("boot"),
  /** The system's languages as the webview sees them, for Rust's own texts (i18n.rs). */
  setSystemLanguages: (languages: string[]) => call<void>("set_system_languages", { languages }),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /** Shrink the window down to the invisible wake strip (hidden) or back to full. */
  setCollapsed: (collapsed: boolean) => call<void>("set_collapsed", { collapsed }),

  /**
   * Pushes the island shape in window coordinates. Rust flips click-through from
   * its own cursor poll, so the flag is never a frame behind a click.
   */
  setIslandRect: (x: number, y: number, width: number, height: number) =>
    call<void>("set_island_rect", { x, y, width, height }),

  /** Give the window keyboard focus (chat field) and take it away again. */
  focusWindow: (focused: boolean) => call<void>("focus_window", { focused }),

  reposition: () => call<void>("reposition"),

  /** Displays the island can be pinned to: `key` is what `settings.screen` stores. */
  listMonitors: () => call<{ key: string; label: string }[]>("list_monitors"),

  openUrl: (url: string) => call<void>("open_url", { url }),

  /** Which of these sounds have a file of the user's own in the sounds folder. */
  customSounds: (names: readonly string[]) => call<string[]>("custom_sounds", { names }),
  /** The user's file for one sound, as raw bytes. */
  customSound: (name: string) => call<ArrayBuffer>("custom_sound", { name }),
  revealSoundsFolder: () => call<void>("reveal_sounds_folder"),
  /** Tells the island to read the sounds folder again ("sounds-changed"). */
  reloadSounds: () => call<void>("reload_sounds"),

  /** "Open terminal" → opens the folder in VS Code when `code` is on PATH. */
  openInVSCode: (path: string | null) => call<boolean>("open_in_vscode", { path }),
  /** A project in the code editor (Antigravity, else VS Code). */
  openProject: (path: string) => callOrThrow<string>("open_project", { path }),
  codeEditorName: () => call<string>("code_editor_name"),
  listCodeEditors: () => call<{ name: string; path: string }[]>("list_code_editors"),

  /**
   * "Open terminal": the window the session runs in when Rust found it
   * (Windows; Linux on X11 and KDE Plasma), else the folder in VS Code.
   */
  openSession: (sessionId: string | null, path: string | null) =>
    call<boolean>("open_session", { sessionId, path }),

  /** The Claude desktop app, for the Claude Desktop pill (Windows only). */
  openClaudeDesktop: () => call<boolean>("open_claude_desktop"),

  /** The diff card's ↗: an existing file, in VS Code; never launched by its type. */
  openFileInVSCode: (path: string) => call<boolean>("open_file_in_vscode", { path }),

  quit: () => call<void>("quit_app"),

  openSettingsWindow: () => call<void>("open_settings_window"),

  /** Writes to %LOCALAPPDATA%\OpenJarvis\openjarvis.log, next to the Rust lines. */
  log: (message: string) => call<void>("log_line", { message }),

  // ── Claude Code hooks ─────────────────────────────────────────────────────
  hooksStatus: () => call<HookStatus>("hooks_status"),
  /** Pill ID → whether that agent's hooks reach OpenJarvis (read-only, Mac #183). */
  agentHooksStatus: () => call<Record<string, boolean>>("agent_hooks_status"),
  /** Diff to show before anything is written. `install: false` previews removal. */
  hooksPreview: (install: boolean) => callOrThrow<HookPreview>("hooks_preview", { install }),
  /**
   * Writes ~/.claude/settings.json — only ever after an explicit click, and only
   * when the file still matches the preview the user looked at.
   */
  hooksApply: (install: boolean, fingerprint: string) =>
    callOrThrow<string>("hooks_apply", { install, fingerprint }),

  // ── Other agents (Gemini CLI, Codex, Cursor…) ─────────────────────────────
  agentHooksList: () => call<AgentHookStatus[]>("agent_hooks_list"),
  /** Diff to show before anything is written. `install: false` previews removal. */
  agentHooksPreview: (agent: string, install: boolean) =>
    callOrThrow<AgentHookPlan>("agent_hooks_preview", { agent, install }),
  /**
   * Writes the agent's config — only ever after an explicit click, and only when
   * it still matches the preview the user looked at. Returns the backups taken.
   */
  agentHooksApply: (agent: string, install: boolean, fingerprint: string) =>
    callOrThrow<string>("agent_hooks_apply", { agent, install, fingerprint }),
  // ── Plan usage: the status line relay, installed apart from the hooks ──────
  /** Diff of the status line change. `install: false` previews taking the relay out. */
  statusLinePreview: (install: boolean) => callOrThrow<HookPreview>("status_line_preview", { install }),
  /** Same rules as hooksApply: an explicit click, and only for the diff that was shown. */
  statusLineApply: (install: boolean, fingerprint: string) =>
    callOrThrow<string>("status_line_apply", { install, fingerprint }),
  /**
   * Asks the Codex CLI (`codex app-server`) for its plan limits, as Codex's
   * /status does. The raw `account/rateLimits/read` result, or null when Codex
   * is missing, not signed in or slow (15 s).
   */
  codexPlanUsage: () => call<unknown>("codex_plan_usage"),

  approvalDecision: (requestId: string, decision: "allow" | "deny") =>
    call<void>("approval_decision", { requestId, decision }),
  /** "The card is up" — until this lands the relay only waits a moment. */
  approvalAck: (requestId: string) => call<void>("approval_ack", { requestId }),
  /** Answers a question Claude Code asked: question text → chosen label. */
  approvalAnswer: (requestId: string, answers: Record<string, string | string[]>) =>
    call<void>("approval_answer", { requestId, answers }),

  /** "Nobody can act on this" — Claude Code asks in the terminal right away. */
  approvalDecline: (requestId: string) => call<void>("approval_decline", { requestId }),

  // ── Chat, files, secrets ──────────────────────────────────────────────────
  /** One chat turn. The API key and any file bytes never leave Rust. */
  chatSend: (query: string, context: ChatContext | null) =>
    callOrThrow<{ text: string }>("chat_send", { query, context }),
  chatReset: () => call<void>("chat_reset"),
  /**
   * The models a provider offers, for the picker in the chat view. Rust asks
   * the provider only when it has a key (or a server address).
   */
  chatModels: (provider: string) => callOrThrow<ModelInfo[]>("chat_models", { provider }),
  /** Settings → Local models → Connect: does the server answer, and with which models? */
  /** The custom server's key, bound to the address it is entered for. */
  localSetKey: (url: string, key: string) => call<void>("local_set_key", { url, key }),

  localConnect: (provider: "ollama" | "lmstudio" | "custom", url: string) =>
    callOrThrow<LocalServer>("local_connect", { provider, url }),
  /** Copies a dropped file into the inbox. */
  ingestFile: (path: string) => callOrThrow<DroppedFile>("ingest_file", { path }),
  /** Only ever tells you whether a key exists — never its value. */
  secretPresent: (key: string) => call<boolean>("secret_present", { key }),
  secretSet: (key: string, value: string) => callOrThrow<void>("secret_set", { key, value }),
  secretClear: (key: string) => callOrThrow<void>("secret_clear", { key }),

  // ── Integrations ──────────────────────────────────────────────────────────
  refreshIntegration: (id: string) => call<void>("refresh_integration", { id }),
  /** The GitHub card is on screen: refetch that part if it is stale. */
  githubRefresh: (section: "pulse" | "activity") => call<void>("github_refresh", { section }),
  /** Opens the configured n8n instance in the browser. */
  openN8n: () => call<void>("open_n8n"),

  /** Tray → Pause. Stops the integration pollers, not just the island. */
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),

  // ── Global shortcuts ──────────────────────────────────────────────────────
  /** How each global shortcut went when Rust last registered them. */
  shortcutsStatus: () => call<ShortcutsReport>("shortcuts_status"),
  /** Lets go of every global shortcut while Settings records a new one. */
  shortcutsSuspend: (suspended: boolean) => call<void>("shortcuts_suspend", { suspended }),

  // ── Weekly recap ──────────────────────────────────────────────────────────
  /** Turns and decisions from `since` (Unix seconds) on, plus the recap prefs. */
  recapHistory: (since: number) => call<RecapHistory>("recap_history", { since: Math.floor(since) }),
  recapPrefs: () => call<RecapPrefs>("recap_prefs"),
  recapSetEnabled: (enabled: boolean) => call<void>("recap_set_enabled", { enabled }),
  recapSetHideProjects: (hide: boolean) => call<void>("recap_set_hide_projects", { hide }),
  /** `week` is the Monday (YYYY-MM-DD) the recap opened on its own for. */
  recapMarkShown: (week: string) => call<void>("recap_mark_shown", { week }),
  recapClear: () => call<void>("recap_clear"),
  /** Writes the PNG (a data URL) into Pictures or Downloads; returns its path. */
  recapSavePng: (data: string, week: string) => callOrThrow<string>("recap_save_png", { data, week }),
  /** Opens the folder of the image saved last. */
  recapRevealSaved: () => call<void>("recap_reveal_saved"),

  // ── Mochi on the desktop (src-tauri/src/desktop.rs) ───────────────────────
  desktopInfo: () => call<DesktopInfo>("desktop_mochi_info"),
  /** Dragged out of the island: (x, y) is the pointer in island-window coordinates. */
  desktopPickUp: (x: number, y: number) => call<boolean>("desktop_mochi_pick_up", { x, y }),
  /** Linux: the pointer moved during that drag (Windows carries him from Rust). */
  desktopCarry: (x: number, y: number) => call<void>("desktop_mochi_carry", { x, y }),
  desktopCarryEnd: (x: number, y: number) => call<void>("desktop_mochi_carry_end", { x, y }),
  /** A drag started on the desktop Mochi; resolves to his top-left corner. */
  desktopDragBegin: () => call<[number, number] | null>("desktop_mochi_drag_begin"),
  /** X11: top-left corner, physical pixels. */
  desktopDragMove: (x: number, y: number) => call<void>("desktop_mochi_drag_move", { x, y }),
  desktopDragEnd: (x: number, y: number) => call<void>("desktop_mochi_drag_end", { x, y }),
  /** From the island to his spot. False: no spot on any connected display.
   *  `anywhere`: the first-visit corner then, rather than staying home. */
  desktopFlyOut: (anywhere = false) => call<boolean>("desktop_mochi_fly_out", { anywhere }),
  /** To the island, then hidden. `forget`: he lives in the island again. */
  desktopFlyHome: (forget: boolean) => call<boolean>("desktop_mochi_fly_home", { forget }),
  /** Asleep, the cursor poll stops. */
  desktopSetAsleep: (asleep: boolean) => call<void>("desktop_mochi_set_asleep", { asleep }),

  // ── Spotify (src-tauri/src/spotify.rs, Linux) ─────────────────────────────
  /** Reads the player again (the card came on screen) and reports it. */
  spotifyRefresh: () => call<SpotifyState | null>("spotify_refresh"),
  /** True when Spotify took it. `value`: seconds for seek, 0/1 for shuffle and repeat, 0…100 for volume. */
  spotifyControl: (action: SpotifyAction, value?: number) =>
    call<boolean>("spotify_control", { action, value: value ?? null }),
  /** Brings Spotify forward or starts it; without it, its download page. */
  spotifyOpen: () => call<boolean>("spotify_open"),
  /** Whether there is a Spotify to launch (Settings). */
  spotifyInstalled: () => call<boolean>("spotify_installed"),

  // ── Jarvis Brain & PC Control ─────────────────────────────────────────────
  openDashboardWindow: () => call<void>("open_dashboard_window"),
  jarvisBrainStatus: () => call<JarvisBrainStatus>("jarvis_brain_status"),
  jarvisAsk: (
    prompt: string,
    history: { role: string; content: string }[],
    modeOverride?: string,
    imageBase64?: string,
    agent?: string,
  ) =>
    callOrThrow<JarvisReply>("jarvis_ask", {
      prompt,
      history,
      modeOverride: modeOverride ?? null,
      imageBase64: imageBase64 ?? null,
      agent: agent ?? null,
    }),
  jarvisExecuteConfirmedAction: (actionId: string, approved: boolean) =>
    callOrThrow<ActionExecutionResult>("jarvis_execute_confirmed_action", { actionId, approved }),
  jarvisListPendingActions: () =>
    call<PendingAction[]>("jarvis_list_pending_actions"),
  jarvisCliLogin: (provider: "claude" | "gemini") =>
    callOrThrow<string>("jarvis_cli_login", { provider }),
  jarvisSaveAccountToken: (provider: "claude" | "gemini", token: string) =>
    callOrThrow<void>("jarvis_save_account_token", { provider, token }),
  jarvisListAccounts: (provider?: string) =>
    call<AiAccount[]>("jarvis_list_accounts", { provider: provider ?? null }),
  jarvisLoginAccount: (provider: AiProviderId, label?: string) =>
    callOrThrow<AiAccount>("jarvis_login_account", { provider, label: label ?? null }),
  jarvisSwitchAccount: (accountId: string) =>
    callOrThrow<AiAccount[]>("jarvis_switch_account", { accountId }),
  jarvisRemoveAccount: (accountId: string) =>
    callOrThrow<AiAccount[]>("jarvis_remove_account", { accountId }),
  jarvisAddAccountManual: (provider: AiProviderId, token: string, label?: string, authMethod?: string) =>
    callOrThrow<AiAccount>("jarvis_add_account_manual", {
      provider,
      label: label ?? null,
      token,
      authMethod: authMethod ?? null,
    }),
  jarvisSendToOpenbot: (command: string, project?: string, agent?: string) =>
    callOrThrow<string>("jarvis_send_to_openbot", {
      command,
      project: project ?? null,
      agent: agent ?? null,
    }),
  jarvisTranscribe: (audioBase64: string, mimeType: string) =>
    callOrThrow<string>("jarvis_transcribe", { audioBase64, mimeType }),
  jarvisTts: (text: string) => callOrThrow<string>("jarvis_tts", { text }),
  jarvisOpenTarget: (target: string) => callOrThrow<string>("jarvis_open_target", { target }),
  jarvisMediaControl: (action: string) => callOrThrow<string>("jarvis_media_control", { action }),
  jarvisCaptureScreen: () => callOrThrow<string>("jarvis_capture_screen"),
  jarvisGetClipboard: () => call<string>("jarvis_get_clipboard"),
  jarvisPcStats: () => call<PcStats>("jarvis_pc_stats"),
  jarvisNowPlaying: () => call<NowPlaying>("jarvis_now_playing"),
  jarvisDataLoad: () => call<JarvisData>("jarvis_data_load"),
  jarvisDataSave: (data: JarvisData) => call<void>("jarvis_data_save", { data }),
  jarvisScanProjects: () => call<JarvisData>("jarvis_scan_projects"),

  // ── Spotify Web API (Premium) — the in-Jarvis Spotify panel ────────────────
  spotifyApiStatus: () => call<SpotifyApiStatus>("spotify_api_status"),
  spotifyLogin: () => callOrThrow<string>("spotify_login"),
  spotifyLogout: () => call<void>("spotify_logout"),
  spotifySearch: (query: string, kinds?: string) => callOrThrow<SpotifyItem[]>("spotify_search", { query, kinds }),
  spotifyMyPlaylists: () => callOrThrow<SpotifyItem[]>("spotify_my_playlists"),
  spotifyCollectionTracks: (uri: string) => callOrThrow<SpotifyItem[]>("spotify_collection_tracks", { uri }),
  spotifyDevices: () => callOrThrow<SpotifyDevice[]>("spotify_devices"),
  spotifyPlayUri: (uri: string, context?: string) => callOrThrow<void>("spotify_play_uri", { uri, context }),
  spotifyPlayerState: () => callOrThrow<SpotifyPlayer | null>("spotify_player_state"),
  spotifyApiControl: (action: string, value?: number, uri?: string) =>
    callOrThrow<void>("spotify_api_control", { action, value, uri }),
  spotifyRecent: () => callOrThrow<SpotifyItem[]>("spotify_recent"),
  spotifyLikedCount: () => callOrThrow<number>("spotify_liked_count"),
  spotifyPlayList: (uris: string[], offset: number) => callOrThrow<void>("spotify_play_list", { uris, offset }),

  // ── Google Calendar — the Agenda tab ──────────────────────────────────────
  calendarStatus: () => call<CalendarStatus>("calendar_status"),
  calendarLogin: () => callOrThrow<string>("calendar_login"),
  calendarLogout: () => call<void>("calendar_logout"),
  /** `from` is a local date (YYYY-MM-DD). */
  calendarEvents: (from: string, days: number) => callOrThrow<CalEvent[]>("calendar_events", { from, days }),
  /** `start`/`end` are local wall clock (YYYY-MM-DDTHH:MM, or YYYY-MM-DD when all day). */
  calendarCreate: (ev: { title: string; start: string; end?: string; allDay?: boolean; location?: string; description?: string; remind?: number | null }) =>
    callOrThrow<CalEvent>("calendar_create", ev),
  calendarUpdate: (key: string, changes: { title?: string; start?: string; end?: string }) =>
    callOrThrow<string>("calendar_update", { key, ...changes }),
  calendarDelete: (key: string) => callOrThrow<void>("calendar_delete", { key }),
  // ── Notch widgets ─────────────────────────────────────────────────────────
  /** Weather for the configured city, or for `city` when testing one. */
  weatherNow: (city?: string) => callOrThrow<WeatherNow>("weather_now", { city: city ?? null }),
  onPcMetrics: (fn: (m: PcMetrics) => void) => onEvent<PcMetrics>("pc-metrics", fn),
  netSpeedTest: () => callOrThrow<number>("net_speed_test"),
  // ── Local wake word (the user's enrolled "Jarvis") ─────────────────────────
  wakewordStatus: () => call<WakeStatus>("wakeword_status"),
  // ── WhatsApp inside Jarvis (whatsapp.rs) ─────────────────────────────────
  whatsappState: () => callOrThrow<WaState>("whatsapp_state"),
  whatsappChats: () => callOrThrow<WaChat[]>("whatsapp_chats"),
  whatsappThread: (chat: string) => callOrThrow<WaThread>("whatsapp_thread", { chat }),
  whatsappSend: (chat: string, text: string) => callOrThrow<string>("whatsapp_send", { chat, text }),
  wakewordRecordSample: (index: number) => callOrThrow<number>("wakeword_record_sample", { index }),
  wakewordBuild: () => callOrThrow<WakeStatus>("wakeword_build"),
  wakewordReset: () => call<WakeStatus>("wakeword_reset"),
  wakewordListenNow: () => callOrThrow<void>("wakeword_listen_now"),
  wakewordRestart: () => call<void>("wakeword_restart"),
  onVoiceLocalState: (fn: (state: "listening" | "transcribing" | "idle") => void) => onEvent<"listening" | "transcribing" | "idle">("voice-local-state", fn),
  onVoiceLocal: (fn: (v: { text: string; error: string }) => void) => onEvent<{ text: string; error: string }>("voice-local", fn),
  // ── In-app updates (GitHub Releases, signature-checked) ──────────────────
  updateCheck: () => callOrThrow<UpdateInfo>("update_check"),
  updateInstall: () => callOrThrow<void>("update_install"),
  onUpdateAvailable: (fn: (u: UpdateInfo) => void) => onEvent<UpdateInfo>("update-available", fn),
  onUpdateProgress: (fn: (p: { downloaded: number; total: number; phase: "downloading" | "installing" }) => void) =>
    onEvent<{ downloaded: number; total: number; phase: "downloading" | "installing" }>("update-progress", fn),
  /** A line in the app log (voice diagnostics). */
  jarvisLog: (line: string) => call<void>("jarvis_log", { line }),
  setVolume: (percent: number) => call<string>("jarvis_set_volume", { percent }),
  // ── Gmail: only when the user opens the E-mail tab or asks ───────────────
  gmailList: (query: string) => callOrThrow<MailItem[]>("gmail_list", { query }),
  gmailRead: (id: string) => callOrThrow<MailFull>("gmail_read", { id }),
  // ── Audio, media of any app, selection ───────────────────────────────────
  audioState: () => callOrThrow<AudioState>("audio_state"),
  audioSetOutput: (id: string) => callOrThrow<void>("audio_set_output", { id }),
  audioSetMicMute: (mute: boolean) => callOrThrow<void>("audio_set_mic_mute", { mute }),
  audioSetMute: (mute: boolean) => callOrThrow<void>("audio_set_mute", { mute }),
  mediaNow: () => call<MediaNow | null>("media_now"),
  mediaControl: (action: string) => callOrThrow<string>("media_control", { action }),
  selectedText: () => call<string | null>("jarvis_selected_text"),
  copyText: (text: string) => call<void>("jarvis_copy_text", { text }),
  // ── Files, notes, routines, briefing, GitHub ─────────────────────────────
  filesSearch: (query: string) => callOrThrow<FileHit[]>("files_search", { query }),
  filesOpen: (path: string, reveal: boolean) => callOrThrow<string>("files_open", { path, reveal }),
  filesWarm: () => call<void>("files_warm"),
  notesSearch: (query: string) => callOrThrow<NoteHit[]>("notes_search", { query }),
  notesRead: (path: string) => callOrThrow<string>("notes_read", { path }),
  notesWrite: (title: string, text: string, folder?: string) => callOrThrow<string>("notes_write", { title, text, folder: folder ?? null }),
  notesOpen: (path: string) => callOrThrow<string>("notes_open", { path }),
  runRoutine: (name: string) => callOrThrow<string>("jarvis_run_routine", { name }),
  briefing: () => call<string>("jarvis_briefing"),
  githubOverview: () => callOrThrow<GithubOverview>("jarvis_github_overview"),
};

export interface WeatherNow {
  place: string;
  temp: number;
  feels: number;
  max: number;
  min: number;
  rainChance: number;
  code: number;
  isDay: boolean;
  kind: "clear" | "partly" | "cloudy" | "fog" | "drizzle" | "rain" | "snow" | "storm";
  label: string;
}

export interface WaState {
  /** "ready" | "qr" | "loading" */
  state: string;
  /** The QR code to scan (data: URL), while not linked. */
  qr: string | null;
}

export interface WaChat {
  title: string;
  last: string;
  time: string;
  unread: number;
}

export interface WaMessage {
  when: string;
  from: string;
  text: string;
  mine: boolean;
}

export interface WaThread {
  title: string;
  messages: WaMessage[];
}

export interface UpdateInfo {
  current: string;
  /** null when already on the latest version. */
  version: string | null;
  notes: string;
  date: string;
}

export interface WakeStatus {
  active: boolean;
  enrolled: boolean;
  samples: number;
  samplesWanted: number;
  error: string;
}

export interface MailItem {
  id: string;
  threadId: string;
  from: string;
  subject: string;
  date: string;
  snippet: string;
  unread: boolean;
}
export interface MailFull extends MailItem {
  to: string;
  body: string;
  messageId: string;
}
export interface AudioDevice {
  id: string;
  name: string;
  default: boolean;
}
export interface AudioState {
  outputs: AudioDevice[];
  volume: number;
  muted: boolean;
  micMuted: boolean | null;
  micName: string;
}
export interface MediaNow {
  title: string;
  artist: string;
  playing: boolean;
  app: string;
}
export interface FileHit {
  path: string;
  name: string;
  dir: string;
  isDir: boolean;
  size: number;
  modified: number;
}
export interface NoteHit {
  path: string;
  title: string;
  snippet: string;
  modified: number;
}
export interface GithubPr {
  id: string;
  title: string;
  url: string;
  repo: string;
  number: number;
  isDraft: boolean;
  ci: "pending" | "success" | "failure" | "unknown";
  review: string;
}
export interface GithubOverview {
  login: string;
  myPRs: GithubPr[];
  toReview: GithubPr[];
  mainCI: { repo: string; url: string; branch: string; ci: GithubPr["ci"] }[];
}

/** `pc-metrics` event, once a second while a PC widget is on. */
export interface PcMetrics {
  cpu: number;
  ram: number;
  ramUsedGb: number;
  ramTotalGb: number;
  gpu: number | null;
  disk: number;
  diskFreeGb: number;
  diskTotalGb: number;
  cpuName: string;
  gpuName: string;
  fps: number | null;
  fpsApp: string;
  fpsAvailable: boolean;
  pingMs: number | null;
  online: boolean;
  netOn: boolean;
  cpuTemp: number | null;
  gpuTemp: number | null;
  sensors: boolean;
  /** The notch is on the other monitor because a game is fullscreen. */
  movedForGame: boolean;
}

export interface CalendarStatus {
  configured: boolean;
  connected: boolean;
  email: string;
  /** Google Cloud project number of the OAuth client in use. */
  project: string;
}

export interface CalEvent {
  /** "calendarId|eventId". */
  key: string;
  title: string;
  /** RFC 3339 for timed events, YYYY-MM-DD for all-day ones. */
  start: string;
  end: string;
  allDay: boolean;
  location: string;
  description: string;
  calendar: string;
  color: string;
  link: string;
  meet: string;
  /** Popup reminders, minutes before the start. */
  reminders?: number[];
}

export interface SpotifyApiStatus {
  clientIdSet: boolean;
  connected: boolean;
  redirectUri: string;
}

export interface SpotifyItem {
  kind: "track" | "album" | "playlist" | "artist";
  uri: string;
  id: string;
  name: string;
  subtitle: string;
  image: string;
  durationMs: number;
}

export interface SpotifyDevice {
  id: string;
  name: string;
  kind: string;
  active: boolean;
  volume: number | null;
}

export interface SpotifyPlayer {
  playing: boolean;
  title: string;
  artist: string;
  album: string;
  image: string;
  uri: string;
  contextUri: string;
  progressMs: number;
  durationMs: number;
  shuffle: boolean;
  repeat: "off" | "context" | "track";
  volume: number | null;
  device: string;
}

export interface NowPlaying {
  title: string;
  artist: string;
  album: string;
  playing: boolean;
  coverUrl: string;
}

export interface PcStats {
  cpuPercent: number;
  memUsedGb: number;
  memTotalGb: number;
  memPercent: number;
  diskFreeGb: number;
  diskTotalGb: number;
  diskPercent: number;
}

export interface MemoryNode {
  id: string;
  label: string;
  category: string;
  notes?: string;
  createdAt: number;
}

export interface MemoryEdge {
  from: string;
  to: string;
  label: string;
}

export interface TaskItem {
  id: string;
  title: string;
  done: boolean;
  project?: string;
  createdAt: number;
}

export interface ReminderItem {
  id: string;
  title: string;
  time: string;
  done: boolean;
}

export interface HabitItem {
  id: string;
  name: string;
  streak: number;
  doneToday: boolean;
}

export interface RoutineItem {
  id: string;
  name: string;
  description: string;
  targets: string[];
  /** "" manual, "app:valorant", "time:18:00". */
  trigger?: string;
}

export interface JarvisData {
  userName: string;
  assistantName?: string;
  accentColor: string;
  mochiColor?: string;
  mochiOutfit?: string;
  /** "solid" | "gradient" | "metallic" | "neon". */
  mochiFinish?: string;
  level?: number;
  xp?: number;
  memories: MemoryNode[];
  edges?: MemoryEdge[];
  tasks: TaskItem[];
  reminders: ReminderItem[];
  habits: HabitItem[];
  routines: RoutineItem[];
}

export interface AgentOption {
  id: string;
  label: string;
  description: string;
}

export interface PendingAction {
  id: string;
  kind: string;
  command: string;
  description: string;
  riskLevel: string;
}

export interface ActionExecutionResult {
  id: string;
  executed: boolean;
  success: boolean;
  output: string;
}

export type AiProviderId = "claude" | "gemini" | "openai" | "groq";

export interface AiAccount {
  id: string;
  provider: AiProviderId;
  label: string;
  authMethod: "oauth" | "token" | "api_key";
  email?: string | null;
  active: boolean;
  createdAt: number;
  expiresAt?: number | null;
}

export interface JarvisBrainStatus {
  groqConfigured: boolean;
  geminiConfigured: boolean;
  openaiConfigured?: boolean;
  claudeConfigured?: boolean;
  ollamaUrl: string;
  brainMode: string;
  claudeCliInstalled: boolean;
  claudeCliLoggedIn: boolean;
  geminiCliInstalled: boolean;
  geminiCliLoggedIn: boolean;
  claudeOauthConfigured: boolean;
  geminiOauthConfigured: boolean;
  availableProviders: string[];
  availableAgents: AgentOption[];
  accounts?: AiAccount[];
}

export interface JarvisReply {
  text: string;
  providerUsed: string;
  actions: string[];
  pendingActions: PendingAction[];
}

export type SpotifyAction = "playPause" | "next" | "previous" | "seek" | "shuffle" | "repeat" | "volume";

export type ShortcutStatus =
  | "active" | "off" | "inUse" | "duplicate" | "invalid"
  | "typesCharacter" | "unsupported" | "notPorted"
  /** Wayland: handed to the desktop's portal, no answer yet / left out by it. */
  | "pending" | "refused";

export interface ShortcutsReport {
  /** `trigger`: on Wayland, the keys the desktop says run it, in its words. */
  actions: { id: string; status: ShortcutStatus; typed?: string; trigger?: string }[];
  /** "wayland" or "no-display" when no key grab can be made. */
  blocked: string | null;
  /** `<executable> --shortcut`: append an action id for a desktop shortcut. */
  command: string;
  /** Wayland: the shortcuts went to the desktop's GlobalShortcuts portal. */
  portal?: "pending" | "active" | null;
}

/** How the desktop Mochi's window works here (platform::DesktopMode). */
export type DesktopMode = "poll" | "window" | "layer" | "off";

export interface DesktopInfo {
  mode: DesktopMode;
  /** He was on the desktop when the app last quit. */
  onDesktop: boolean;
}

/** An event for one window only (island ⇄ desktop Mochi). Never throws. */
export async function emitToWindow(label: string, event: string, payload?: unknown) {
  if (!IS_TAURI) return;
  try {
    await emitTo(label, event, payload);
  } catch (err) {
    console.error(`[openjarvis] emit ${event} failed`, err);
  }
}

export interface IntegrationUpdate {
  id: string;
  data: Record<string, unknown>;
  error: string | null;
  event: { success: boolean; label: string; detail: string | null } | null;
}

export type ChatContext =
  | { kind: "file"; name: string; path: string }
  | { kind: "window"; appName: string; title: string; url?: string };

export interface ModelInfo {
  id: string;
  label: string;
}

export interface LocalServer {
  /** The address as it is stored. */
  url: string;
  models: string[];
  /** The address is this machine. */
  loopback: boolean;
}

export interface DroppedFile {
  name: string;
  path: string;
  size: number;
}

export interface HookStatus {
  installed: boolean;
  /** OpenJarvis's status line relay (plan usage) is the status line in settings.json. */
  planRelayInstalled: boolean;
  settingsPath: string;
  hookPath: string;
  hookReady: boolean;
}

/** One agent other than Claude Code, as agents.rs reports it. */
export interface AgentHookStatus {
  /** The `--agent` name; its pill is `agent_<id>`. */
  id: string;
  name: string;
  installed: boolean;
  /** The file (or files, one per line) OpenJarvis writes. */
  path: string;
  hookReady: boolean;
  /** The island can allow or deny this agent's permission requests. */
  approvals: boolean;
  /** What to do once it is written. */
  note: string;
}

export interface AgentHookPlan {
  diff: string;
  /** Where each existing file is copied first, one per line; "" when none. */
  backup: string;
  path: string;
  fingerprint: string;
}

export interface HookPreview {
  diff: string;
  backup: string;
  settingsPath: string;
  /** Hand back to hooksApply so only the reviewed diff is ever written. */
  fingerprint: string;
}

/** Same as `call`, but surfaces the error so the UI can show what went wrong. */
async function callOrThrow<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside OpenJarvis");
  return invoke<T>(cmd, args);
}

export type BridgeEvent =
  | { name: "cursor"; payload: { x: number; y: number } }
  | { name: "tray"; payload: string }
  | { name: "hook"; payload: Record<string, unknown> }
  | { name: "screen-changed"; payload: null };

export interface DragDropPayload {
  type: "enter" | "over" | "drop" | "leave";
  paths?: string[];
}

interface WebView2Bridge {
  postMessageWithAdditionalObjects(message: unknown, objects: ArrayLike<unknown>): void;
}

/**
 * Files dragged onto the island. Only reaches us when the window takes the mouse.
 *
 * On Windows WebView2 takes the drop itself, as in Edge — Tauri's drop handling
 * never sees drags from the classic Explorer folder view (see
 * src-tauri/src/webview_drop.rs). Enter/over/leave come straight from the page;
 * the drop hands the File objects to Rust, which answers with their real paths
 * as a `file-drag` event. Elsewhere (Linux) Tauri's own drag events still do it.
 */
export async function onDragDrop(handler: (e: DragDropPayload) => void) {
  if (!IS_TAURI) return () => {};
  const webview = (window as unknown as { chrome?: { webview?: WebView2Bridge } }).chrome?.webview;
  if (!webview) {
    return getCurrentWebview().onDragDropEvent((event) => {
      handler(event.payload as DragDropPayload);
    });
  }
  const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes("Files") ?? false;
  // dragenter/dragleave fire for every element crossed; only the outermost pair counts.
  let depth = 0;

  const onEnter = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    if (depth++ === 0) handler({ type: "enter" });
  };
  const onOver = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    // Without this WebView2 refuses the drop — or navigates to the file.
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    handler({ type: "over" });
  };
  const onLeave = (e: DragEvent) => {
    if (!hasFiles(e) || depth === 0) return;
    if (--depth === 0) handler({ type: "leave" });
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    depth = 0;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) {
      handler({ type: "drop", paths: [] });
      return;
    }
    webview.postMessageWithAdditionalObjects("openjarvis-file-drop", files);
  };

  window.addEventListener("dragenter", onEnter);
  window.addEventListener("dragover", onOver);
  window.addEventListener("dragleave", onLeave);
  window.addEventListener("drop", onDrop);
  const unlisten = await listen<DragDropPayload>("file-drag", (e) => handler(e.payload));
  return () => {
    window.removeEventListener("dragenter", onEnter);
    window.removeEventListener("dragover", onOver);
    window.removeEventListener("dragleave", onLeave);
    window.removeEventListener("drop", onDrop);
    unlisten();
  };
}

export async function onEvent<T>(name: string, handler: (payload: T) => void) {
  if (!IS_TAURI) return () => {};
  return listen<T>(name, (e) => handler(e.payload));
}
