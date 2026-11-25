import "./notch.css";
import "./notch_v2.css";
import "./notch_motion.css";
import "./glass.css";
import {
  Bridge,
  onEvent,
  type AgentOption,
  type AiAccount,
  type AiProviderId,
  type JarvisBrainStatus,
  type JarvisData,
  type MemoryNode,
  type NowPlaying,
  type PcStats,
  type PendingAction,
} from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type Settings } from "../core/state";
import { BODY_FINISHES, BotEngine, hexToRGB, type BodyFinish } from "../mochi/engine";
import type { Outfit } from "../mochi/wardrobe";
import { CATEGORY_META, renderMemoryGraph } from "../dashboard/memory_graph";
import { clear, h } from "../views/dom";
import { Voice, asksBack, listenCue } from "./voice";
import { createMusicPanel } from "./music_panel";
import { createAgendaPanel, eventSpan } from "./agenda_panel";
import { createNotchWidgets } from "./notch_widgets";
import { createToolTabs } from "./tool_tabs";
import type { MediaNow, PcMetrics, WaCall } from "../core/bridge";
import type { CalEvent } from "../core/bridge";

type TabId =
  | "home"
  | "chat"
  | "memory"
  | "tasks"
  | "reminders"
  | "habits"
  | "timers"
  | "music"
  | "agenda"
  | "clipboard"
  | "routines"
  | "nicknames"
  | "pc"
  | "mail"
  | "whatsapp"
  | "files"
  | "notes"
  | "settings";

type OrganizeSubTab = "tasks" | "reminders" | "timers" | "habits" | "routines" | "nicknames" | "clipboard";
type SettingsSubTab = "brain" | "customize" | "notch";
type ProjectsViewMode = "list" | "graph";

interface PendingActionItem extends PendingAction {
  status: "pending" | "running" | "approved" | "cancelled" | "error";
  resultOutput?: string;
}

interface ChatTurn {
  role: "user" | "assistant";
  content: string;
  meta?: string;
  pendingActions?: PendingActionItem[];
}

const DEFAULT_AGENTS: AgentOption[] = [
  { id: "orquestrador", label: "Orquestrador", description: "Coordena demandas, planeja e delega entre especialistas" },
  { id: "frontend-mobile", label: "Frontend & Mobile", description: "React, Next.js, TypeScript, Vite, Tailwind e React Native" },
  { id: "backend-dados", label: "Backend & Dados", description: "APIs, Python, Node/TS, Rust, Supabase, PostgreSQL e SQLite" },
  { id: "devops", label: "DevOps & Infra", description: "Docker, CI/CD, deploy, servidores, Git e automação" },
  { id: "ui-ux", label: "UI/UX & Design", description: "Interface, usabilidade, CSS, animações e design system" },
];

interface ActiveTimer {
  id: string;
  label: string;
  remainingSec: number;
  running: boolean;
}

interface LiveMochi {
  canvas: HTMLCanvasElement;
  engine: BotEngine;
  size: number;
}

const liveMochis = new Map<HTMLCanvasElement, LiveMochi>();

function createDressedMochi(
  bodySize: number,
  colorHex: string,
  outfit: Outfit,
  state: "idle" | "working" | "thinking" = "idle",
): HTMLElement {
  const slot = document.createElement("span");
  slot.style.width = `${bodySize}px`;
  slot.style.height = `${bodySize}px`;
  slot.style.display = "inline-flex";
  slot.style.alignItems = "center";
  slot.style.justifyContent = "center";
  slot.style.overflow = "visible";

  const canvas = document.createElement("canvas");
  const engineSize = Math.round(bodySize / 0.56);
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  canvas.width = Math.round(engineSize * dpr);
  canvas.height = Math.round(engineSize * dpr);
  canvas.style.width = `${engineSize}px`;
  canvas.style.height = `${engineSize}px`;
  slot.append(canvas);

  const engine = new BotEngine();
  engine.isMini = false;
  engine.bodyColor = hexToRGB(colorHex || "#f4f4f5");
  engine.bodyFinish = mochiFinish;
  engine.setState(state, true);
  engine.setOutfit(outfit, false);

  liveMochis.set(canvas, { canvas, engine, size: engineSize });
  return slot;
}

function pruneDressedMochis() {
  for (const [c] of liveMochis) {
    if (!c.isConnected) liveMochis.delete(c);
  }
}

function tickDressedMochis(dt: number, colorHex: string, outfit: Outfit, state: "idle" | "working" | "thinking") {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  for (const m of liveMochis.values()) {
    const ctx = m.canvas.getContext("2d");
    if (!ctx) continue;
    m.engine.bodyColor = hexToRGB(colorHex || "#f4f4f5");
    m.engine.bodyFinish = mochiFinish;
    m.engine.setOutfit(outfit, true);
    m.engine.setState(state);
    m.engine.update(dt);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, m.size, m.size);
    m.engine.draw(ctx, m.size, m.size);
  }
}

function svgIcon(pathD: string, size = 14, filled = false): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "nav-icon";
  wrap.style.width = `${size}px`;
  wrap.style.height = `${size}px`;
  wrap.innerHTML = filled
    ? `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="currentColor"><path d="${pathD}"/></svg>`
    : `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="${pathD}"/></svg>`;
  return wrap;
}

const ICONS = {
  home: "M3 9.5L12 3l9 6.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1V9.5z",
  chat: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
  projects: "M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z",
  music: "M9 18V5l12-2v13 M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M18 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  organize: "M9 11l3 3L22 4 M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11",
  pc: "M2 4h20v12H2z M8 20h8 M12 16v4",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.09a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.09a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
  mic: "M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z M19 10v2a7 7 0 0 1-14 0v-2 M12 19v4 M8 23h8",
  stop: "M6 6h12v12H6z",
  eye: "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
  send: "M22 2L11 13 M22 2l-7 20-4-9-9-4 20-7z",
  mail: "M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M22 6l-10 7L2 6",
  whatsapp: "M3 21l1.65-4.94A9 9 0 1 1 8 19.6L3 21z M9 9.5c0 3 2.5 5.5 5.5 5.5l1.2-1.2-1.8-1-1 .8a4 4 0 0 1-2.3-2.3l.8-1-1-1.8L9 9.5z",
  fileSearch: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h5 M14 2l6 6 M14 2v6h6 M16.5 19.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z M18.5 18.5L21 21",
  notes: "M12 20h9 M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z",
  prev: "M19 20L9 12l10-8v16z M5 19V5",
  next: "M5 4l10 8-10 8V4z M19 5v14",
  play: "M6 4l14 8-14 8V4z",
  pause: "M6 4h4v16H6z M14 4h4v16h-4z",
  volDown: "M11 5L6 9H2v6h4l5 4V5z M15.54 8.46a5 5 0 0 1 0 7.07",
  volUp: "M11 5L6 9H2v6h4l5 4V5z M15.54 8.46a5 5 0 0 1 0 7.07 M19.07 4.93a10 10 0 0 1 0 14.14",
  external: "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6 M15 3h6v6 M10 14L21 3",
  sync: "M23 4v6h-6 M1 20v-6h6 M3.51 9a9 9 0 0 1 14.85-3.36L23 10 M1 14l4.64 4.36A9 9 0 0 0 20.49 15",
  close: "M18 6L6 18 M6 6l12 12",
  chevronUp: "M18 15l-6-6-6 6",
  calendar: "M4 5h16v16H4z M4 10h16 M9 3v4 M15 3v4",
};

const OUTFIT_GROUPS: { title: string; items: { id: Outfit; label: string }[] }[] = [
  {
    title: "Chapéus",
    items: [
      { id: "baseballCap", label: "Boné" },
      { id: "silkHat", label: "Cartola" },
      { id: "cowboyHat", label: "Cowboy" },
      { id: "pirateHat", label: "Pirata" },
      { id: "chefHat", label: "Chef" },
      { id: "gradCap", label: "Capelo" },
      { id: "vikingHelmet", label: "Viking" },
      { id: "santaHat", label: "Gorro Natal" },
      { id: "beanie", label: "Touca" },
      { id: "partyHat", label: "Festa" },
      { id: "witchHat", label: "Bruxa" },
      { id: "crown", label: "Coroa" },
    ],
  },
  {
    title: "Na cabeça",
    items: [
      { id: "headphones", label: "Fone" },
      { id: "headset", label: "Headset gamer" },
      { id: "flowerCrown", label: "Flores" },
      { id: "catEars", label: "Gato" },
      { id: "bunnyEars", label: "Coelho" },
      { id: "halo", label: "Auréola" },
      { id: "devilHorns", label: "Chifrinhos" },
      { id: "ninjaBand", label: "Ninja" },
      { id: "bow", label: "Laço" },
    ],
  },
  {
    title: "No rosto",
    items: [
      { id: "sunglasses", label: "Óculos" },
      { id: "roundGlasses", label: "Redondos" },
      { id: "mask", label: "Máscara" },
      { id: "mustache", label: "Bigode" },
      { id: "scarf", label: "Cachecol" },
      { id: "none", label: "Nada" },
    ],
  },
];

const FINISH_OPTIONS: { id: BodyFinish; label: string }[] = [
  { id: "solid", label: "Sólido" },
  { id: "gradient", label: "Degradê" },
  { id: "metallic", label: "Metálico" },
  { id: "neon", label: "Neon" },
];

/** The finish every dressed Mochi is painted with (from data.mochiFinish). */
let mochiFinish: BodyFinish = "solid";

const ORGANIZE_TABS: Set<TabId> = new Set([
  "tasks",
  "reminders",
  "timers",
  "habits",
  "routines",
  "nicknames",
  "clipboard",
]);

function extractProjectPath(notes?: string): string | null {
  const m = (notes ?? "").match(/Caminho:\s*([^\n]+)/);
  return m ? m[1].trim() : null;
}

export async function startJarvisNotch(root: HTMLElement) {
  let expanded = false;
  let activeTab: TabId = "home";
  let organizeSubTab: OrganizeSubTab = "tasks";
  let settingsSubTab: SettingsSubTab = "brain";
  let authSubTab: "accounts" | "apikeys" = "accounts";
  let authNotice = "";
  let accounts: AiAccount[] = [];
  let loggingInProvider: AiProviderId | null = null;
  let selectedAgent = localStorage.getItem("jarvis_selected_agent") || "orquestrador";
  let projectsViewMode: ProjectsViewMode = "list";
  let brainStatus: JarvisBrainStatus | null = null;
  let data: JarvisData = {
    // Filled from the store (the Windows user name until one is set).
    userName: "",
    assistantName: "Jarvis",
    accentColor: "#84cc16",
    mochiColor: "#f4f4f5",
    mochiOutfit: "santaHat",
    level: 1,
    xp: 35,
    memories: [],
    edges: [],
    tasks: [],
    reminders: [],
    habits: [],
    routines: [],
  };
  let pcStats: PcStats | null = null;
  let nowPlaying: NowPlaying = { title: "", artist: "", album: "", playing: false, coverUrl: "" };
  let pcScanStep = -1;
  let selectedMemory: MemoryNode | null = null;
  let memoryFilter = "";
  let memoryCatFilter = "";
  let sending = false;
  let liveStatus: { phase: "idle" | "thinking" | "executing"; title: string; detail: string } = {
    phase: "idle",
    title: "",
    detail: "",
  };
  let toastReply = "";
  /** A WhatsApp call in progress or ringing (the collapsed notch shows it). */
  let waCall: WaCall | null = null;
  let toastTimer: number | null = null;
  let cleanupGraph: (() => void) | null = null;
  let lastClip = "";

  const chatLog: ChatTurn[] = [
    {
      role: "assistant",
      content: "Olá! Tudo pronto por aqui. Como posso ajudar?",
    },
  ];
  const clipboardHistory: string[] = [];
  const timers: ActiveTimer[] = [];

  // Today's Google Calendar events, for Home (refreshed every minute).
  let todayEvents: CalEvent[] = [];
  const agendaPanel = createAgendaPanel({
    onToday(evts) {
      todayEvents = evts;
    },
  });
  async function refreshTodayEvents() {
    try {
      const evts = await agendaPanel.today();
      const changed = JSON.stringify(evts) !== JSON.stringify(todayEvents);
      todayEvents = evts;
      if (changed && expanded && activeTab === "home" && !(document.activeElement instanceof HTMLInputElement)) render();
    } catch {
      // Home just shows tasks when the calendar cannot be read.
    }
  }

  // Created before anything can call render(): it is one long-lived element.
  const musicPanel = createMusicPanel({
    async saveClientId(id: string) {
      await saveSettings({ ...State.settings, jarvisSpotifyClientId: id });
    },
    onTrackChange(p) {
      if (!p) return;
      nowPlaying = { title: p.title, artist: p.artist, album: p.album, playing: p.playing, coverUrl: p.image };
    },
  });

  // Weather / PC rings / FPS in the pill: one long-lived element too.
  const widgets = createNotchWidgets(() => {
    if (!expanded) pushVisibleRect();
  });
  let widgetSettings: Settings | null = null;
  const toolTabs = createToolTabs({
    rerender: () => render(),
    isOpen: (tab) => expanded && activeTab === tab,
    sectionHead: (t, a) => sectionHead(t, a),
    ask: (prompt) => {
      activeTab = "chat";
      void askJarvis(prompt);
    },
    toast: (text) => showToast(text, 6000),
  });
  /** Media playing in another app (YouTube, VLC…), for the pill. */
  let mediaNow: MediaNow | null = null;
  /** A game in the foreground (FPS seen): for game mode and its summary. */
  let game: { app: string; start: number; fps: number[]; misses: number } | null = null;
  let soundBeforeGame: boolean | null = null;
  const gameModeOn = () => Boolean(game && State.settings.jarvisGameMode);
  let lastView = "";
  /** A newer version on GitHub (app_update.rs), and the download's progress. */
  let updateInfo: import("../core/bridge").UpdateInfo | null = null;
  let updateState: { phase: "idle" | "checking" | "downloading" | "installing" | "error" | "latest"; pct: number; note: string } = {
    phase: "idle",
    pct: 0,
    note: "",
  };
  /** Local wake word status (wakeword.rs), and a way to reload it. */
  let wakeStatus: import("../core/bridge").WakeStatus | null = null;
  let refreshWakeSoon: () => void = () => {};
  /** Enrollment in progress: which sample, and what to tell the user. */
  let enroll: { index: number; note: string; err: boolean; busy: boolean } | null = null;
  /** A voice command heard while the previous request was still running. */
  let queuedVoice: { text: string; screen: boolean } | null = null;

  /** The code editor projects open in (Antigravity when installed). */
  let editorName = "editor";
  void Bridge.codeEditorName().then((n) => {
    if (n && n !== editorName) {
      editorName = n;
      render();
    }
  });

  /** Jarvis asked a question out loud: listen for the answer, no "Jarvis" needed.
   *  Quiet for ~6 s and it gives up on its own. */
  function listenForAnswer() {
    window.setTimeout(() => {
      // Something else started meanwhile (a new command, a recording).
      if (sending || ["listening", "recording", "transcribing"].includes(Voice.state)) return;
      void Bridge.jarvisLog("voice: ouvindo a resposta (o Jarvis fez uma pergunta)");
      if (Voice.localWake) void Bridge.wakewordListenNow().catch(() => Voice.toggleRecording());
      else void Voice.toggleRecording();
    }, 250); // the end of his own voice must not get into the recording
  }
  /** Last "Salvar" result under the city field, kept across re-renders. */
  let weatherNote: { text: string; err: boolean } | null = null;

  void Sound.preload();
  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  applyTheme();
  Sound.setEnabled(State.settings.soundEnabled);
  Sound.setVolume(State.settings.soundVolume);

  const loaded = await Bridge.jarvisDataLoad();
  if (loaded) data = { ...data, ...loaded };
  brainStatus = await Bridge.jarvisBrainStatus();
  accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
  void onEvent<AiAccount[]>("jarvis-accounts-updated", (updated) => {
    if (Array.isArray(updated)) {
      accounts = updated;
      void Bridge.jarvisBrainStatus().then((st) => {
        if (st) brainStatus = st;
        render();
      });
    }
  });
  const initialNp = await Bridge.jarvisNowPlaying();
  if (initialNp) nowPlaying = initialNp;

  async function refreshMedia(delayMs = 0) {
    if (delayMs > 0) await new Promise((r) => setTimeout(r, delayMs));
    const np = await Bridge.jarvisNowPlaying();
    if (!np) return;
    const changed =
      np.title !== nowPlaying.title ||
      np.artist !== nowPlaying.artist ||
      np.playing !== nowPlaying.playing ||
      np.coverUrl !== nowPlaying.coverUrl;
    nowPlaying = np;
    if (changed) {
      const typing = document.activeElement instanceof HTMLInputElement;
      if (!typing && (!expanded || activeTab === "home" || activeTab === "music")) {
        render();
      }
    }
  }

  function currentOutfit(): Outfit {
    return (data.mochiOutfit as Outfit) || "santaHat";
  }

  function hasRunningAction(): PendingActionItem | undefined {
    for (const m of chatLog) {
      const running = m.pendingActions?.find((a) => a.status === "running");
      if (running) return running;
    }
    return undefined;
  }

  function currentBotState(): "idle" | "working" | "thinking" {
    if (
      Voice.state === "recording" ||
      Voice.state === "listening" ||
      Voice.state === "transcribing" ||
      liveStatus.phase === "executing" ||
      hasRunningAction()
    ) {
      return "working";
    }
    if (sending || liveStatus.phase === "thinking") return "thinking";
    return "idle";
  }

  function pushRect(el: HTMLElement) {
    requestAnimationFrame(() => {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) {
        void Bridge.setIslandRect(
          Math.max(0, r.left - 4),
          0,
          r.width + 8,
          // From the top edge (the "Ilha" shape floats a few pixels below it).
          r.bottom + 6,
        );
      }
    });
  }

  /** Re-reports whatever is on screen. Rust decides where the window takes
   *  clicks from this rectangle, so it must follow every change of layout —
   *  not only renders: growing back from the hidden strip (240 px wide) to the
   *  full window moves the pill to the new centre. */
  function pushVisibleRect() {
    const el = root.querySelector<HTMLElement>(".notch-panel, .notch-voice-bar, .notch-pill, .wake-strip");
    if (el) pushRect(el);
  }
  window.addEventListener("resize", pushVisibleRect);
  // The pill slides in (transform): report its final place once it lands.
  root.addEventListener("animationend", pushVisibleRect);

  function openTab(tab: TabId) {
    activeTab = tab;
    if (ORGANIZE_TABS.has(tab)) {
      organizeSubTab = tab as OrganizeSubTab;
    }
    render();
  }

  // ── AI status (sidebar) ─────────────────────────────────────────────────
  /** Last error from the brain; cleared by the next answer. */
  let aiError = "";
  window.addEventListener("online", () => render());
  window.addEventListener("offline", () => render());

  const MODE_NAMES: Record<string, string> = {
    gemini: "Gemini",
    "gemini-oauth": "Gemini (conta)",
    groq: "Groq",
    openai: "ChatGPT / OpenAI",
    chatgpt: "ChatGPT / OpenAI",
    claude: "Claude",
    "claude-oauth": "Claude (conta)",
    "claude-cli": "Claude CLI",
    "gemini-cli": "Gemini CLI",
    ollama: "Ollama",
  };

  /** Ready / offline (the chosen AI cannot answer now) / none (no AI set up). */
  function aiHealth(): { state: "ready" | "offline" | "none"; label: string; detail: string } {
    const st = brainStatus;
    const mode = State.settings.jarvisBrainMode || "auto";
    const has: Record<string, boolean> = {
      gemini: Boolean(st?.geminiConfigured || st?.geminiOauthConfigured),
      "gemini-oauth": Boolean(st?.geminiOauthConfigured),
      groq: Boolean(st?.groqConfigured),
      openai: Boolean(st?.openaiConfigured),
      chatgpt: Boolean(st?.openaiConfigured),
      claude: Boolean(st?.claudeConfigured || st?.claudeOauthConfigured),
      "claude-oauth": Boolean(st?.claudeOauthConfigured),
      "claude-cli": Boolean(st?.claudeCliInstalled),
      "gemini-cli": Boolean(st?.geminiCliInstalled),
      ollama: true, // a local server: reachable or not, it is set up
    };
    const anyCloud = ["gemini", "groq", "openai", "claude", "claude-cli", "gemini-cli"].some((k) => has[k]);
    const name = MODE_NAMES[mode] ?? "IA";

    if (mode === "auto" ? !anyCloud : !has[mode]) {
      return mode === "auto"
        ? { state: "none", label: "Sem IA configurada", detail: "Conecte uma IA em Ajustes → IA & Voz." }
        : { state: "none", label: `${name} não conectado`, detail: `O modo escolhido é ${name}, mas não há conta conectada. Abra Ajustes → IA & Voz.` };
    }
    if (mode !== "ollama" && !navigator.onLine) {
      return { state: "offline", label: "IA offline", detail: "Sem conexão com a internet." };
    }
    if (aiError) {
      return { state: "offline", label: "IA offline", detail: `A última resposta falhou: ${aiError}` };
    }
    return { state: "ready", label: "Pronto", detail: mode === "auto" ? "IA automática pronta" : `${name} pronto` };
  }

  // ── Window states: hidden strip ↔ pill ↔ panel, with open/close motion ──
  /** "Escondido": only an invisible strip at the top until hover or voice. */
  const notchHidden = () => (State.settings.jarvisNotchMode ?? "always") === "hidden";
  /** In hidden mode: the pill is showing because the mouse came by. */
  let revealed = false;
  /** The OS window is shrunk to the wake strip (no cursor polling at all). */
  let windowCollapsed = false;
  let collapseTimer: number | null = null;
  let hideTimer: number | null = null;
  /** The next render plays the opening / appearing motion. */
  let animateOpen = false;
  let animatePill = false;
  let closing = false;

  function setWindowCollapsed(next: boolean) {
    if (collapseTimer != null) {
      window.clearTimeout(collapseTimer);
      collapseTimer = null;
    }
    if (next) {
      // After the pill's exit motion, drop to the strip.
      collapseTimer = window.setTimeout(() => {
        collapseTimer = null;
        if (!windowCollapsed) {
          windowCollapsed = true;
          void Bridge.setCollapsed(true);
        }
      }, 220);
    } else if (windowCollapsed) {
      windowCollapsed = false;
      // Once the window is full size again, the pill sits somewhere else:
      // report it (and again a little later, in case the resize event is late).
      void Bridge.setCollapsed(false).then(() => {
        pushVisibleRect();
        window.setTimeout(pushVisibleRect, 120);
      });
    }
  }

  function setExpanded(next: boolean, tab?: TabId) {
    if (tab) {
      activeTab = tab;
      if (ORGANIZE_TABS.has(tab)) organizeSubTab = tab as OrganizeSubTab;
    }
    if (expanded === next) {
      if (tab) render();
      return;
    }
    if (!next) {
      // Closing: play the panel's exit, then switch to the compact state.
      if (closing) return;
      const panel = root.querySelector<HTMLElement>(".notch-panel");
      if (panel) {
        closing = true;
        panel.classList.remove("opening");
        panel.classList.add("closing");
        window.setTimeout(() => {
          closing = false;
          finishExpand(false);
        }, 170);
        return;
      }
    }
    finishExpand(next);
  }

  function finishExpand(next: boolean) {
    if (expanded === next) return;
    expanded = next;
    if (expanded) {
      toastReply = "";
      animateOpen = true;
      setWindowCollapsed(false);
    } else {
      // Hidden mode: closing the panel hides the notch again.
      revealed = false;
    }
    Sound.play("pop");
    void Bridge.focusWindow(expanded);
    render();
  }

  // Clicking anywhere outside the panel (another app, the desktop) takes focus
  // away from the notch window: that closes the panel.
  window.addEventListener("blur", () => {
    if (expanded && !closing) setExpanded(false);
  });

  async function saveData() {
    await Bridge.jarvisDataSave(data);
  }

  async function saveSettings(s: Settings) {
    State.settings = s;
    applyTheme();
    await Bridge.saveSettings(s);
  }

  /** The chosen look (glass.css): classes on <html>, applied at once. */
  function applyTheme() {
    const t = State.settings.jarvisTheme || "graphite";
    const cl = document.documentElement.classList;
    cl.toggle("glass", ["glass", "midnight", "aurora", "emerald"].includes(t));
    for (const v of ["midnight", "aurora", "emerald"]) cl.toggle(v, t === v);
    const shape = State.settings.jarvisNotchShape || "notch";
    cl.toggle("shape-island", shape === "island");
    cl.toggle("shape-tab", shape === "tab");
    // The window's clickable area follows the new shape.
    requestAnimationFrame(() => pushVisibleRect());
  }

  function getAgentsList(): AgentOption[] {
    return brainStatus?.availableAgents?.length ? brainStatus.availableAgents : DEFAULT_AGENTS;
  }

  function setSelectedAgent(agentId: string) {
    selectedAgent = agentId || "orquestrador";
    localStorage.setItem("jarvis_selected_agent", selectedAgent);
  }

  async function handleConfirmAction(item: PendingActionItem, approved: boolean) {
    if (item.status !== "pending") return;
    item.status = "running";
    render();
    try {
      const res = await Bridge.jarvisExecuteConfirmedAction(item.id, approved);
      if (!approved) {
        item.status = "cancelled";
        item.resultOutput = res.output || "Ação cancelada pelo usuário.";
      } else if (res.success) {
        item.status = "approved";
        item.resultOutput = res.output || "Executado com sucesso.";
        Sound.play("finish");
      } else {
        item.status = "error";
        item.resultOutput = res.output || "Falha ao executar ação.";
        Sound.play("error");
      }
      const fresh = await Bridge.jarvisDataLoad();
      if (fresh) data = { ...data, ...fresh };
    } catch (err) {
      item.status = "error";
      item.resultOutput = String(err).replace(/^Error:\s*/, "");
      Sound.play("error");
    } finally {
      render();
    }
  }

  /** A short message in the compact notch (routines, "bom dia", hotkeys). */
  function showToast(text: string, ms = 7000) {
    if (!text.trim()) return;
    toastReply = text;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toastReply = "";
      render();
    }, ms);
    if (!expanded) render();
  }

  async function askJarvis(prompt: string, withScreen = false, stayCompact = false, viaVoice = false): Promise<string | undefined> {
    const q = prompt.trim();
    if (!q || sending) return;
    let answer: string | undefined;
    sending = true;
    chatLog.push({ role: "user", content: withScreen ? `[Tela] ${q}` : q });
    render();

    try {
      const img = withScreen ? await Bridge.jarvisCaptureScreen() : undefined;
      const history = chatLog.slice(-10).map((m) => ({ role: m.role, content: m.content }));
      const reply = await Bridge.jarvisAsk(q, history, undefined, img, selectedAgent);
      aiError = "";
      const actionsNote = reply.actions.length ? ` · ${reply.actions.join(", ")}` : "";
      const pendingItems: PendingActionItem[] = (reply.pendingActions ?? []).map((p) => ({
        ...p,
        status: "pending",
      }));
      chatLog.push({
        role: "assistant",
        content: reply.text,
        meta: `${reply.providerUsed}${actionsNote}`,
        pendingActions: pendingItems.length ? pendingItems : undefined,
      });
      Sound.play("finish");
      // Asked by voice and he asks back: the answer is heard right after he speaks.
      const followUp = viaVoice && !pendingItems.length && asksBack(reply.text);
      void Voice.speak(reply.text).then(() => {
        if (followUp) listenForAnswer();
      });
      answer = reply.text;
      void Bridge.jarvisLog(
        `resposta (${reply.providerUsed})${reply.actions.length ? ` · ações: ${reply.actions.join(" | ")}` : " · sem ação"}: "${reply.text.slice(0, 120)}"`,
      );

      if (pendingItems.length > 0 && !expanded) {
        setExpanded(true, "chat");
      } else if (stayCompact && !expanded) {
        toastReply = reply.text;
        if (toastTimer) clearTimeout(toastTimer);
        toastTimer = window.setTimeout(() => {
          toastReply = "";
          render();
        }, 7000);
      }

      const fresh = await Bridge.jarvisDataLoad();
      if (fresh) data = { ...data, ...fresh };
    } catch (err) {
      const msg = String(err).replace(/^Error:\s*/, "");
      void Bridge.jarvisLog(`erro no pedido "${q.slice(0, 80)}": ${msg.slice(0, 200)}`);
      // The brain did not answer: the sidebar status says so until one does.
      aiError = msg;
      chatLog.push({ role: "assistant", content: msg, meta: "Erro" });
      if (stayCompact && !expanded) toastReply = msg;
      Sound.play("error");
    } finally {
      sending = false;
      liveStatus = { phase: "idle", title: "", detail: "" };
      render();
    }
    // A voice command that arrived meanwhile.
    if (queuedVoice) {
      const next = queuedVoice;
      queuedVoice = null;
      window.setTimeout(() => void askJarvis(next.text, next.screen, !expanded, true), 50);
    }
    return answer;
  }

  function buildPromptBar(placeholder: string, onScreenClick?: () => void) {
    const input = h("input", { placeholder }) as HTMLInputElement;
    const isRec = Voice.state === "recording";

    const sendBtn = h(
      "button",
      {
        class: "circle-btn primary",
        title: "Enviar",
        onclick: () => {
          const val = input.value.trim();
          if (!val) return;
          input.value = "";
          void askJarvis(val);
        },
      },
      svgIcon(ICONS.send, 13),
    );

    const micBtn = h(
      "button",
      {
        class: `circle-btn ${isRec ? "recording" : ""}`,
        title: isRec ? "Parar gravação" : "Falar comando",
        onclick: () => void Voice.toggleRecording(),
      },
      svgIcon(isRec ? ICONS.stop : ICONS.mic, 13),
    );

    const visionBtn = h(
      "button",
      {
        class: "circle-btn",
        title: "Analisar tela",
        onclick: () => {
          if (onScreenClick) {
            onScreenClick();
          } else {
            const val = input.value.trim() || "Analise o que está na minha tela agora.";
            input.value = "";
            void askJarvis(val, true);
          }
        },
      },
      svgIcon(ICONS.eye, 13),
    );

    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") sendBtn.click();
    });

    return { bar: h("div", { class: "prompt-bar" }, input, visionBtn, micBtn, sendBtn), input };
  }

  function buildCompactSpotifyCard(): HTMLElement {
    const coverEl = nowPlaying.coverUrl
      ? h("div", { class: "spotify-cover" }, h("img", { src: nowPlaying.coverUrl }))
      : h("div", { class: "spotify-cover" }, svgIcon(ICONS.music, 17));

    const statusEl = nowPlaying.playing
      ? h(
          "div",
          { class: "spotify-status playing" },
          h("span", { class: "wave eq" }, h("i"), h("i"), h("i")),
          h("span", { text: "Tocando" }),
        )
      : h(
          "div",
          { class: "spotify-status" },
          h("span", { text: nowPlaying.title ? "Pausado" : "Inativo" }),
        );

    return h(
      "div",
      { class: "card spotify-card" },
      h(
        "div",
        { class: "spotify-top" },
        coverEl,
        h(
          "div",
          { class: "spotify-meta" },
          h("div", { class: "spotify-title", text: nowPlaying.title || "Spotify" }),
          h("div", {
            class: "spotify-artist",
            text: nowPlaying.artist
              ? `${nowPlaying.artist}${nowPlaying.album ? ` · ${nowPlaying.album}` : ""}`
              : "Nenhuma faixa tocando",
          }),
        ),
        h(
          "button",
          {
            class: "icon-action",
            title: "Abrir Spotify",
            onclick: () => void Bridge.jarvisOpenTarget("spotify"),
          },
          svgIcon(ICONS.external, 13),
        ),
      ),
      h(
        "div",
        { class: "spotify-footer" },
        statusEl,
        h(
          "div",
          { class: "spotify-controls" },
          h(
            "button",
            {
              class: "icon-action",
              title: "Voltar",
              onclick: () => {
                void Bridge.playerControl("prev", nowPlaying.title).catch((e) => showToast(String(e)));
                void refreshMedia(450);
              },
            },
            svgIcon(ICONS.prev, 13, true),
          ),
          h(
            "button",
            {
              class: "play-circle",
              title: nowPlaying.playing ? "Pausar" : "Tocar",
              onclick: () => {
                void Bridge.playerControl("play_pause", nowPlaying.title).catch((e) => showToast(String(e)));
                void refreshMedia(350);
              },
            },
            svgIcon(nowPlaying.playing ? ICONS.pause : ICONS.play, 13, true),
          ),
          h(
            "button",
            {
              class: "icon-action",
              title: "Avançar",
              onclick: () => {
                void Bridge.playerControl("next", nowPlaying.title).catch((e) => showToast(String(e)));
                void refreshMedia(450);
              },
            },
            svgIcon(ICONS.next, 13, true),
          ),
          h(
            "button",
            {
              class: "icon-action",
              title: "Diminuir volume",
              onclick: () => void Bridge.jarvisMediaControl("vol_down"),
            },
            svgIcon(ICONS.volDown, 13),
          ),
          h(
            "button",
            {
              class: "icon-action",
              title: "Aumentar volume",
              onclick: () => void Bridge.jarvisMediaControl("vol_up"),
            },
            svgIcon(ICONS.volUp, 13),
          ),
        ),
      ),
    );
  }

  // ── 1. Início ──────────────────────────────────────────────────────────────

  function sectionHead(title: string, action?: { label: string; onclick: () => void }): HTMLElement {
    return h(
      "div",
      { class: "sec-head" },
      h("span", { class: "sec-title", text: title }),
      action ? h("button", { class: "link-btn", text: action.label, onclick: action.onclick }) : "",
    );
  }

  function renderHome(main: HTMLElement) {
    const { bar } = buildPromptBar("Peça algo: “toca Dawn FM”, “abre o YouTube”, “volume 40”…", () => {
      activeTab = "chat";
      void askJarvis("Analise o que está na minha tela agora.", true);
    });

    const now = new Date();
    const dateLabel = now.toLocaleDateString("pt-BR", { weekday: "long", day: "numeric", month: "long" });
    const hour = now.getHours();
    const greet = hour < 12 ? "Bom dia" : hour < 18 ? "Boa tarde" : "Boa noite";

    // Today: reminders first (by time), then open tasks.
    const today = h("div", { class: "list" });
    // Calendar events first: they have fixed times.
    for (const e of todayEvents.slice(0, 4)) {
      today.append(
        h(
          "button",
          { class: "list-row link-row", title: "Abrir na Agenda", onclick: () => openTab("agenda") },
          h("span", { class: "list-time", text: e.allDay ? "Dia" : eventSpan(e).split("–")[0] }),
          h("i", { class: "ag-bar sm", style: `background:${e.color || "var(--text-3)"}` }),
          h("span", { class: "list-text", text: e.title }),
        ),
      );
    }
    const reminders = data.reminders.filter((r) => !r.done).sort((a, b) => a.time.localeCompare(b.time));
    const openTasks = data.tasks.filter((t) => !t.done);
    for (const r of reminders.slice(0, 3)) {
      today.append(
        h(
          "div",
          { class: "list-row" },
          h("span", { class: "list-time", text: r.time }),
          h("span", { class: "list-text", text: r.title }),
        ),
      );
    }
    const shown = Math.min(4, todayEvents.length) + Math.min(3, reminders.length);
    for (const t of openTasks.slice(0, Math.max(0, 6 - shown))) {
      const check = h("button", { class: "check", title: "Concluir" });
      check.addEventListener("click", async () => {
        t.done = true;
        await saveData();
        render();
      });
      today.append(h("div", { class: "list-row" }, check, h("span", { class: "list-text", text: t.title })));
    }
    if (!todayEvents.length && !reminders.length && !openTasks.length) {
      today.append(h("div", { class: "empty", text: "Nada pendente. Diga “lembra de…” para criar um lembrete." }));
    }

    // Recent activity: the last exchanges, newest first, one line each.
    const activity = h("div", { class: "list" });
    const recent = chatLog.filter((m) => m.role === "user" || m.meta).slice(-4).reverse();
    for (const m of recent) {
      activity.append(
        h(
          "div",
          { class: "list-row activity" },
          h("span", { class: `who ${m.role}`, text: m.role === "user" ? "Você" : "Jarvis" }),
          h("span", { class: "list-text", text: m.content }),
        ),
      );
    }
    if (!recent.length) activity.append(h("div", { class: "empty", text: "Nenhuma conversa ainda." }));

    const projects = data.memories.filter((m) => m.category === "project").slice(0, 4);
    const projList = h("div", { class: "list" });
    for (const proj of projects) {
      const pPath = extractProjectPath(proj.notes);
      projList.append(
        h(
          "div",
          { class: "list-row" },
          svgIcon(ICONS.projects, 13),
          h("span", { class: "list-text", text: proj.label }),
          pPath
            ? h("button", { class: "link-btn quiet", text: `Abrir no ${editorName}`, onclick: () => void Bridge.openProject(pPath).catch((e) => showToast(String(e).replace(/^Error:\s*/, ""))) })
            : "",
        ),
      );
    }
    if (!projects.length) projList.append(h("div", { class: "empty", text: "Nenhum projeto indexado." }));

    main.append(
      h(
        "header",
        { class: "page-head" },
        h("div", { class: "page-title", text: data.userName.trim() ? `${greet}, ${data.userName.trim()}` : greet }),
        h("div", { class: "page-sub", text: dateLabel.charAt(0).toUpperCase() + dateLabel.slice(1) }),
      ),
      bar,
      h(
        "div",
        { class: "home-grid" },
        h(
          "section",
          { class: "home-col" },
          buildCompactSpotifyCard(),
          sectionHead("Hoje", { label: "Organizar", onclick: () => openTab("tasks") }),
          today,
        ),
        h(
          "section",
          { class: "home-col" },
          sectionHead("Atividade", { label: "Conversa", onclick: () => openTab("chat") }),
          activity,
          sectionHead("Projetos", { label: "Ver todos", onclick: () => openTab("memory") }),
          projList,
        ),
      ),
    );
  }

  // ── 2. Conversa ────────────────────────────────────────────────────────────

  function renderPendingActionCard(item: PendingActionItem): HTMLElement {
    const risk = (item.riskLevel || "").toLowerCase();
    const riskCls = risk === "high" || risk === "danger" ? "danger" : risk === "medium" || risk === "warning" ? "warning" : "";
    const statusLabel =
      item.status === "running"
        ? "Executando…"
        : item.status === "approved"
          ? "Executado"
          : item.status === "cancelled"
            ? "Cancelado"
            : item.status === "error"
              ? "Erro na execução"
              : item.riskLevel || item.kind;

    return h(
      "div",
      { class: `sys-confirm-card ${riskCls}` },
      h(
        "div",
        { class: "sys-confirm-head" },
        h(
          "div",
          { class: "sys-confirm-title" },
          svgIcon(ICONS.pc, 13),
          h("span", { text: "Confirmação de ação no sistema" }),
        ),
        h("span", { class: "sys-confirm-badge", text: `${item.kind} · ${statusLabel}` }),
      ),
      item.description ? h("div", { class: "sys-confirm-desc", text: item.description }) : "",
      h("div", { class: "sys-confirm-cmd", text: item.command }),
      item.status === "pending"
        ? h(
            "div",
            { class: "sys-confirm-actions" },
            h("button", {
              class: "btn btn-sm btn-cancel",
              text: "Cancelar",
              onclick: () => void handleConfirmAction(item, false),
            }),
            h("button", {
              class: "btn btn-sm btn-allow",
              text: "Permitir",
              onclick: () => void handleConfirmAction(item, true),
            }),
          )
        : item.resultOutput
          ? h("div", {
              class: "sys-confirm-desc",
              style: item.status === "error" ? "color:#fca5a5" : item.status === "approved" ? "color:#6ee7b7" : "",
              text: item.resultOutput,
            })
          : "",
    );
  }

  function renderChat(main: HTMLElement) {
    const msgs = h("div", { class: "chat-messages" });
    for (const m of chatLog) {
      msgs.append(
        h(
          "div",
          { class: `msg ${m.role}` },
          h("div", { text: m.content }),
          m.meta ? h("div", { class: "msg-meta", text: m.meta }) : "",
        ),
      );
      if (m.pendingActions?.length) {
        for (const act of m.pendingActions) {
          msgs.append(renderPendingActionCard(act));
        }
      }
    }
    if (sending) msgs.append(h("div", { class: "msg assistant", text: "Pensando…" }));

    const agents = getAgentsList();
    const agentSelect = h(
      "select",
      {
        class: "agent-selector-select",
        title: "Selecionar agente para executar a tarefa",
        onchange: (e: Event) => {
          setSelectedAgent((e.target as HTMLSelectElement).value);
          render();
        },
      },
      ...agents.map((ag) =>
        h("option", {
          value: ag.id,
          text: ag.label,
          selected: ag.id === selectedAgent,
        }),
      ),
    ) as HTMLSelectElement;
    agentSelect.value = selectedAgent;

    const { bar } = buildPromptBar("Mensagem para o Jarvis…");

    main.append(
      h(
        "div",
        { class: "chat-header" },
        h("div", { class: "page-title", text: "Conversa" }),
        h(
          "div",
          { class: "chat-header-right" },
          h(
            "label",
            { class: "agent-selector", title: agents.find((a) => a.id === selectedAgent)?.description ?? "" },
            h("span", { class: "agent-dot" }),
            agentSelect,
          ),
          h(
            "span",
            { class: "row", style: "font-size:11px;color:var(--muted);gap:6px" },
            h("span", { class: "status-dot green" }),
            h("span", { text: sending ? "Processando…" : "Online" }),
          ),
        ),
      ),
      h("div", { class: "chat-wrap" }, msgs, bar),
    );
    requestAnimationFrame(() => {
      msgs.scrollTop = msgs.scrollHeight;
    });
  }

  // ── 3. Projetos & Notas ────────────────────────────────────────────────────

  function renderMemory(main: HTMLElement) {
    const search = h("input", {
      class: "input",
      style: "width:190px",
      placeholder: "Filtrar projetos ou notas…",
      value: memoryFilter,
    }) as HTMLInputElement;
    search.addEventListener("input", () => {
      memoryFilter = search.value;
      if (projectsViewMode === "graph") {
        drawGraph();
      } else {
        renderListIntoContainer();
      }
    });

    const scanBtn = h(
      "button",
      {
        class: "btn btn-ghost btn-sm",
        onclick: async () => {
          const fresh = await Bridge.jarvisScanProjects();
          if (fresh) data = { ...data, ...fresh };
          Sound.play("finish");
          render();
        },
      },
      svgIcon(ICONS.sync, 12),
      h("span", { text: "Sincronizar" }),
    );

    const viewToggle = h(
      "div",
      { class: "row", style: "gap:4px" },
      h("button", {
        class: `filter-chip ${projectsViewMode === "list" ? "on" : ""}`,
        text: "Lista",
        onclick: () => {
          projectsViewMode = "list";
          render();
        },
      }),
      h("button", {
        class: `filter-chip ${projectsViewMode === "graph" ? "on" : ""}`,
        text: "Grafo",
        onclick: () => {
          projectsViewMode = "graph";
          render();
        },
      }),
    );

    const cats: { id: string; label: string }[] = [
      { id: "", label: "Todos" },
      { id: "project", label: "Projetos" },
      { id: "tech", label: "Tecnologias" },
      { id: "person", label: "Pessoas" },
      { id: "fact", label: "Notas" },
    ];
    const chipRow = h("div", { class: "row between" },
      h(
        "div",
        { class: "row", style: "gap:5px" },
        ...cats.map((c) =>
          h("button", {
            class: `filter-chip ${memoryCatFilter === c.id ? "on" : ""}`,
            text: c.label,
            onclick: () => {
              memoryCatFilter = c.id;
              render();
            },
          }),
        ),
      ),
      viewToggle,
    );

    const listContainer = h("div", { class: "project-grid", style: "flex:1;min-height:0" });

    function renderListIntoContainer() {
      clear(listContainer);
      const q = memoryFilter.trim().toLowerCase();
      const items = data.memories.filter((m) => {
        if (m.category === "user" || m.category === "pref") return false;
        if (memoryCatFilter && m.category !== memoryCatFilter) return false;
        if (!q) return true;
        return (
          m.label.toLowerCase().includes(q) ||
          (m.notes ?? "").toLowerCase().includes(q)
        );
      });

      if (items.length === 0) {
        listContainer.append(
          h("div", {
            class: "empty-state",
            style: "grid-column:1 / -1",
            text: "Nenhum item encontrado. Clique em Sincronizar ou adicione abaixo.",
          }),
        );
        return;
      }

      for (const item of items) {
        const meta = CATEGORY_META[item.category] ?? { color: "#9ca3af", label: "Nota" };
        const projectPath = extractProjectPath(item.notes);
        listContainer.append(
          h(
            "div",
            { class: "project-card" },
            h(
              "div",
              {},
              h(
                "div",
                { class: "row between", style: "margin-bottom:4px" },
                h("span", { style: `font-size:10px;font-weight:600;color:${meta.color}`, text: meta.label }),
                h(
                  "button",
                  {
                    class: "icon-action",
                    title: "Remover",
                    onclick: async () => {
                      data.memories = data.memories.filter((m) => m.id !== item.id);
                      data.edges = (data.edges ?? []).filter((e) => e.from !== item.id && e.to !== item.id);
                      await saveData();
                      render();
                    },
                  },
                  svgIcon(ICONS.close, 11),
                ),
              ),
              h("div", { style: "font-weight:600;font-size:12.5px;color:#fff", text: item.label }),
              item.notes
                ? h("div", {
                    style: "font-size:11px;color:var(--muted);margin-top:3px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden",
                    text: item.notes,
                  })
                : "",
            ),
            projectPath
              ? h("button", {
                  class: "btn btn-ghost btn-sm",
                  style: "align-self:flex-start",
                  text: `Abrir no ${editorName}`,
                  onclick: () => void Bridge.openProject(projectPath).catch((e) => showToast(String(e).replace(/^Error:\s*/, ""))),
                })
              : "",
          ),
        );
      }
    }

    const canvas = h("canvas", { class: "graph-canvas" }) as HTMLCanvasElement;
    const inspector = h("div", { class: "graph-inspector", style: "display:none" });
    const wrap = h("div", { class: "graph-wrap" }, canvas, inspector);

    function updateInspector() {
      clear(inspector);
      if (!selectedMemory) {
        inspector.style.display = "none";
        return;
      }
      inspector.style.display = "flex";
      const meta = CATEGORY_META[selectedMemory.category] ?? { color: "#9ca3af", label: "Nota" };
      const projectPath = extractProjectPath(selectedMemory.notes);

      inspector.append(
        h(
          "div",
          { class: "row between" },
          h("span", { style: `font-size:10px;font-weight:600;color:${meta.color}`, text: meta.label }),
          h(
            "button",
            {
              class: "icon-action",
              onclick: () => {
                selectedMemory = null;
                updateInspector();
              },
            },
            svgIcon(ICONS.close, 12),
          ),
        ),
        h("div", { style: "font-weight:600;font-size:13px;color:#fff", text: selectedMemory.label }),
        selectedMemory.notes
          ? h("div", { style: "font-size:11px;color:var(--muted);white-space:pre-wrap;line-height:1.35", text: selectedMemory.notes })
          : "",
        h(
          "div",
          { class: "row", style: "margin-top:4px" },
          projectPath
            ? h("button", {
                class: "btn btn-sm",
                style: "flex:1",
                text: `Abrir no ${editorName}`,
                onclick: () => void Bridge.openProject(projectPath).catch((e) => showToast(String(e).replace(/^Error:\s*/, ""))),
              })
            : "",
          selectedMemory.category !== "user"
            ? h("button", {
                class: "btn btn-danger btn-sm",
                text: "Excluir",
                onclick: async () => {
                  const delId = selectedMemory?.id;
                  data.memories = data.memories.filter((m) => m.id !== delId);
                  data.edges = (data.edges ?? []).filter((e) => e.from !== delId && e.to !== delId);
                  selectedMemory = null;
                  await saveData();
                  render();
                },
              })
            : "",
        ),
      );
    }

    function drawGraph() {
      cleanupGraph?.();
      const filteredMems = memoryCatFilter
        ? data.memories.filter((m) => m.category === "user" || m.category === memoryCatFilter)
        : data.memories;
      cleanupGraph = renderMemoryGraph(
        canvas,
        filteredMems,
        data.edges ?? [],
        memoryFilter,
        (node) => {
          selectedMemory = node;
          updateInspector();
        },
      );
    }

    const catSelect = h(
      "select",
      { class: "input", style: "width:110px" },
      h("option", { value: "project", text: "Projeto" }),
      h("option", { value: "fact", text: "Nota" }),
      h("option", { value: "person", text: "Pessoa" }),
      h("option", { value: "tech", text: "Tecnologia" }),
    ) as HTMLSelectElement;

    const newMemIn = h("input", {
      class: "input",
      style: "flex:1",
      placeholder: "Adicionar nota ou projeto rápido…",
    }) as HTMLInputElement;

    const addMem = async () => {
      const val = newMemIn.value.trim();
      if (!val) return;
      const cat = catSelect.value || "fact";
      const id = `mem_${Date.now()}`;
      data.memories.push({
        id,
        label: val,
        category: cat,
        notes: "",
        createdAt: Math.floor(Date.now() / 1000),
      });
      data.edges = data.edges ?? [];
      data.edges.push({
        from: cat === "project" ? "hub_projects" : "hub_facts",
        to: id,
        label: cat,
      });
      newMemIn.value = "";
      await saveData();
      render();
    };
    newMemIn.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void addMem();
    });

    main.append(
      h(
        "div",
        { class: "row between" },
        h("div", { class: "page-title", text: "Projetos & Notas" }),
        h("div", { class: "row" }, search, scanBtn),
      ),
      chipRow,
      projectsViewMode === "list" ? listContainer : wrap,
      h(
        "div",
        { class: "row" },
        catSelect,
        newMemIn,
        h("button", { class: "btn", text: "Adicionar", onclick: () => void addMem() }),
      ),
    );

    if (projectsViewMode === "list") {
      renderListIntoContainer();
    } else {
      updateInspector();
      requestAnimationFrame(drawGraph);
    }
  }

  // ── 4. Spotify ─────────────────────────────────────────────────────────────

  function renderMusic(main: HTMLElement) {
    main.classList.add("main-flush");
    main.append(musicPanel.el);
    void musicPanel.show();
  }

  // ── 5. Organizar (Tarefas, Lembretes, Timers, Hábitos, Rotinas, Copiados) ─

  function renderOrganize(main: HTMLElement) {
    const subTabs: { id: OrganizeSubTab; label: string }[] = [
      { id: "tasks", label: "Tarefas" },
      { id: "reminders", label: "Lembretes" },
      { id: "timers", label: "Timers" },
      { id: "habits", label: "Hábitos" },
      { id: "routines", label: "Rotinas" },
      { id: "nicknames", label: "Contatos" },
      { id: "clipboard", label: "Copiados" },
    ];

    const subNav = h(
      "div",
      { class: "row", style: "gap:5px;flex-wrap:wrap" },
      ...subTabs.map((t) =>
        h("button", {
          class: `filter-chip ${organizeSubTab === t.id ? "on" : ""}`,
          text: t.label,
          onclick: () => {
            organizeSubTab = t.id;
            activeTab = t.id;
            render();
          },
        }),
      ),
    );

    main.append(
      h("div", { class: "page-title", text: "Organizar" }),
      subNav,
    );

    switch (organizeSubTab) {
      case "tasks": renderTasksBody(main); break;
      case "reminders": renderRemindersBody(main); break;
      case "timers": renderTimersBody(main); break;
      case "habits": renderHabitsBody(main); break;
      case "routines": renderRoutinesBody(main); break;
      case "nicknames": renderNicknamesBody(main); break;
      case "clipboard": renderClipboardBody(main); break;
    }
  }

  function renderTasksBody(main: HTMLElement) {
    const input = h("input", { class: "input", style: "flex:1", placeholder: "Nova tarefa…" }) as HTMLInputElement;
    const add = async () => {
      const title = input.value.trim();
      if (!title) return;
      data.tasks.push({ id: `t_${Date.now()}`, title, done: false, createdAt: Math.floor(Date.now() / 1000) });
      await saveData();
      render();
    };
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") void add(); });

    const list = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const t of data.tasks) {
      list.append(
        h(
          "div",
          { class: "list-item" },
          h(
            "label",
            { class: "row", style: "cursor:pointer" },
            h("input", {
              type: "checkbox",
              checked: t.done,
              onchange: async () => {
                t.done = !t.done;
                await saveData();
                render();
              },
            }),
            h("span", { style: t.done ? "text-decoration:line-through;color:var(--muted)" : "", text: t.title }),
          ),
          h(
            "button",
            {
              class: "icon-action",
              onclick: async () => {
                data.tasks = data.tasks.filter((x) => x.id !== t.id);
                await saveData();
                render();
              },
            },
            svgIcon(ICONS.close, 12),
          ),
        ),
      );
    }
    main.append(
      h("div", { class: "row" }, input, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
      data.tasks.length ? list : h("div", { class: "empty-state", text: "Nenhuma tarefa pendente." }),
    );
  }

  function renderRemindersBody(main: HTMLElement) {
    const titleIn = h("input", { class: "input", style: "flex:1", placeholder: "Lembrete…" }) as HTMLInputElement;
    const timeIn = h("input", { class: "input", type: "time", value: "09:00" }) as HTMLInputElement;
    const add = async () => {
      const title = titleIn.value.trim();
      if (!title) return;
      data.reminders.push({ id: `r_${Date.now()}`, title, time: timeIn.value || "09:00", done: false });
      await saveData();
      render();
    };
    const list = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const r of data.reminders) {
      list.append(
        h(
          "div",
          { class: "list-item" },
          h("span", { text: `${r.time} — ${r.title}` }),
          h(
            "button",
            {
              class: "icon-action",
              onclick: async () => {
                data.reminders = data.reminders.filter((x) => x.id !== r.id);
                await saveData();
                render();
              },
            },
            svgIcon(ICONS.close, 12),
          ),
        ),
      );
    }
    main.append(
      h("div", { class: "row" }, titleIn, timeIn, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
      data.reminders.length ? list : h("div", { class: "empty-state", text: "Nenhum lembrete criado." }),
    );
  }

  function renderHabitsBody(main: HTMLElement) {
    const input = h("input", { class: "input", style: "flex:1", placeholder: "Novo hábito diário…" }) as HTMLInputElement;
    const add = async () => {
      const name = input.value.trim();
      if (!name) return;
      data.habits.push({ id: `h_${Date.now()}`, name, streak: 0, doneToday: false });
      await saveData();
      render();
    };
    const list = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const hb of data.habits) {
      list.append(
        h(
          "div",
          { class: "list-item" },
          h(
            "div",
            { class: "row" },
            h("button", {
              class: hb.doneToday ? "btn btn-sm" : "btn btn-ghost btn-sm",
              text: hb.doneToday ? "Feito" : "Marcar",
              onclick: async () => {
                hb.doneToday = !hb.doneToday;
                hb.streak = Math.max(0, hb.streak + (hb.doneToday ? 1 : -1));
                await saveData();
                render();
              },
            }),
            h("span", { text: `${hb.name} · ${hb.streak}d` }),
          ),
          h(
            "button",
            {
              class: "icon-action",
              onclick: async () => {
                data.habits = data.habits.filter((x) => x.id !== hb.id);
                await saveData();
                render();
              },
            },
            svgIcon(ICONS.close, 12),
          ),
        ),
      );
    }
    main.append(
      h("div", { class: "row" }, input, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
      data.habits.length ? list : h("div", { class: "empty-state", text: "Nenhum hábito cadastrado." }),
    );
  }

  function renderTimersBody(main: HTMLElement) {
    const labelIn = h("input", { class: "input", style: "flex:1", placeholder: "Nome do timer (ex: Foco)" }) as HTMLInputElement;
    const minIn = h("input", { class: "input", type: "number", value: "25", style: "width:76px" }) as HTMLInputElement;
    const list = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const tm of timers) {
      const m = Math.floor(tm.remainingSec / 60).toString().padStart(2, "0");
      const s = (tm.remainingSec % 60).toString().padStart(2, "0");
      list.append(
        h(
          "div",
          { class: "list-item" },
          h("span", { style: "font-weight:600;font-variant-numeric:tabular-nums", text: `${tm.label} — ${m}:${s}` }),
          h(
            "div",
            { class: "row" },
            h("button", {
              class: "btn btn-ghost btn-sm",
              text: tm.running ? "Pausar" : "Retomar",
              onclick: () => {
                tm.running = !tm.running;
                render();
              },
            }),
            h(
              "button",
              {
                class: "icon-action",
                onclick: () => {
                  const idx = timers.indexOf(tm);
                  if (idx >= 0) timers.splice(idx, 1);
                  render();
                },
              },
              svgIcon(ICONS.close, 12),
            ),
          ),
        ),
      );
    }
    main.append(
      h(
        "div",
        { class: "row" },
        labelIn,
        minIn,
        h("button", {
          class: "btn",
          text: "Iniciar",
          onclick: () => {
            const mins = Math.max(1, Number(minIn.value) || 5);
            timers.push({
              id: `tm_${Date.now()}`,
              label: labelIn.value.trim() || `${mins} min`,
              remainingSec: mins * 60,
              running: true,
            });
            render();
          },
        }),
      ),
      timers.length ? list : h("div", { class: "empty-state", text: "Nenhum timer ativo." }),
    );
  }

  function renderRoutinesBody(main: HTMLElement) {
    const nameIn = h("input", { class: "input", style: "width:155px", placeholder: "Nome (ex: Trabalho)" }) as HTMLInputElement;
    const targetsIn = h("input", { class: "input", style: "flex:1", placeholder: "Passos: obs, esperar:5, obs:cena Câmera, obs:gravar, projeto:site, teclas:ctrl+s, digitar:oi, spotify:lofi, volume:30, close:chrome, whatsapp:Bia: chego já, say:Bom jogo!" }) as HTMLInputElement;
    const triggerIn = h(
      "select",
      { class: "input", style: "width:150px;color-scheme:dark" },
      h("option", { value: "", text: "Só manual" }),
      h("option", { value: "app", text: "Ao abrir um app" }),
      h("option", { value: "time", text: "Num horário" }),
    ) as HTMLSelectElement;
    const triggerVal = h("input", { class: "input", style: "width:140px", placeholder: "valorant / 18:00" }) as HTMLInputElement;

    const addRoutine = async () => {
      const name = nameIn.value.trim();
      const rawTargets = targetsIn.value.trim();
      if (!name || !rawTargets) return;
      const targets = rawTargets.split(",").map((s) => s.trim()).filter(Boolean);
      const trigger = triggerIn.value && triggerVal.value.trim() ? `${triggerIn.value}:${triggerVal.value.trim()}` : "";
      data.routines.push({
        id: `rot_${Date.now()}`,
        name,
        description: `${trigger ? (trigger.startsWith("app:") ? `Ao abrir ${trigger.slice(4)} · ` : `Às ${trigger.slice(5)} · `) : ""}${targets.join(", ")}`,
        targets,
        trigger,
      });
      await saveData();
      render();
    };

    const grid = h("div", { class: "grid-2" });
    for (const rt of data.routines) {
      grid.append(
        h(
          "div",
          { class: "card" },
          h(
            "div",
            { class: "row between", style: "margin-bottom:4px" },
            h("div", { style: "font-weight:600", text: rt.name }),
            h(
              "button",
              {
                class: "icon-action",
                onclick: async () => {
                  data.routines = data.routines.filter((x) => x.id !== rt.id);
                  await saveData();
                  render();
                },
              },
              svgIcon(ICONS.close, 12),
            ),
          ),
          h("div", { style: "font-size:11px;color:var(--muted);margin-bottom:9px", text: rt.description }),
          h("button", {
            class: "btn btn-ghost btn-sm",
            text: "Executar",
            onclick: async () => {
              const msg = await Bridge.runRoutine(rt.name).catch((e) => String(e));
              Sound.play("finish");
              showToast(msg);
            },
          }),
        ),
      );
    }
    main.append(
      h("div", { class: "row" }, nameIn, targetsIn),
      h("div", { class: "row", style: "margin-top:6px" }, triggerIn, triggerVal, h("button", { class: "btn", text: "Criar", onclick: () => void addRoutine() })),
      h("div", {
        style: "font-size:11px;color:var(--text-3);line-height:1.5;margin:6px 0 10px",
        text: "Passos separados por vírgula: um programa ou site abre; esperar:5 espera 5 s; teclas:ctrl+shift+n e digitar:texto agem na janela da frente; obs:gravar / obs:live / obs:cena Câmera / obs:mutar controlam o OBS; projeto:nome abre o projeto no editor; whatsapp:Bia: texto pergunta antes de mandar; spotify:, volume:, saida:, mic:, close:, say: como antes. Com gatilho, a rotina roda sozinha quando o app abre (ex.: valorant) ou no horário (ex.: 18:00). Também dá para rodar falando o nome.",
      }),
      data.routines.length ? grid : h("div", { class: "empty-state", text: "Nenhuma rotina criada." }),
    );
  }

  // WhatsApp contacts by the name the user says: "Bia" → "Beatriz Souza".
  function renderNicknamesBody(main: HTMLElement) {
    const nickIn = h("input", { class: "input", style: "width:150px", placeholder: "Apelido (ex: Bia)" }) as HTMLInputElement;
    const contactIn = h("input", { class: "input", style: "flex:1", placeholder: "Contato no WhatsApp (ex: Beatriz Souza)" }) as HTMLInputElement;
    const add = async () => {
      const nick = nickIn.value.trim();
      const contact = contactIn.value.trim();
      if (!nick || !contact) return;
      const key = nick.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
      const list = (data.contactNicknames ?? []).filter(
        (n) => n.nick.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase() !== key,
      );
      list.push({ nick, contact });
      data.contactNicknames = list;
      await saveData();
      render();
    };
    contactIn.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void add();
    });
    const list = data.contactNicknames ?? [];
    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const n of list) {
      rows.append(
        h(
          "div",
          { class: "card row between", style: "padding:9px 12px" },
          h(
            "div",
            { class: "row", style: "gap:8px" },
            h("span", { style: "font-weight:600", text: n.nick }),
            h("span", { style: "color:var(--muted)", text: "→" }),
            h("span", { text: n.contact }),
          ),
          h(
            "button",
            {
              class: "icon-action",
              title: "Apagar",
              onclick: async () => {
                data.contactNicknames = (data.contactNicknames ?? []).filter((x) => x.nick !== n.nick);
                await saveData();
                render();
              },
            },
            svgIcon(ICONS.close, 12),
          ),
        ),
      );
    }
    // Contacts whose new messages Jarvis announces.
    const fold = (s: string) => s.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase().trim();
    const watchIn = h("input", { class: "input", style: "flex:1", placeholder: "Contato ou apelido (ex: Bia)" }) as HTMLInputElement;
    const addWatch = async () => {
      const c = watchIn.value.trim();
      if (!c) return;
      const nick = (data.contactNicknames ?? []).find((n) => fold(n.nick) === fold(c));
      const name = nick ? nick.contact : c;
      data.whatsappWatch = [...(data.whatsappWatch ?? []).filter((w) => fold(w) !== fold(name)), name];
      await saveData();
      render();
    };
    watchIn.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void addWatch();
    });
    const watchRows = h("div", { style: "display:flex;flex-wrap:wrap;gap:6px" });
    for (const w of data.whatsappWatch ?? []) {
      watchRows.append(
        h(
          "div",
          { class: "card row", style: "padding:6px 10px;gap:8px" },
          h("span", { text: `🔔 ${w}` }),
          h(
            "button",
            {
              class: "icon-action",
              title: "Parar de avisar",
              onclick: async () => {
                data.whatsappWatch = (data.whatsappWatch ?? []).filter((x) => x !== w);
                await saveData();
                render();
              },
            },
            svgIcon(ICONS.close, 11),
          ),
        ),
      );
    }
    const watchSection = h(
      "div",
      { style: "margin-top:18px" },
      h("div", { style: "font-weight:600;margin-bottom:6px", text: "Avisar quando mandarem mensagem" }),
      h("div", { class: "row" }, watchIn, h("button", { class: "btn", text: "Avisar", onclick: () => void addWatch() })),
      h("div", {
        style: "font-size:11px;color:var(--text-3);line-height:1.5;margin:6px 0 10px",
        text: "Quando um desses contatos mandar mensagem, o Jarvis avisa e pergunta se pode ler; depois, pergunta o que você quer responder. Por voz: “me avisa quando a Bia mandar mensagem”.",
      }),
      (data.whatsappWatch ?? []).length ? watchRows : h("div", { class: "empty-state", text: "Nenhum contato com aviso." }),
    );

    main.append(
      h("div", { style: "font-weight:600;margin-bottom:6px", text: "Apelidos" }),
      h("div", { class: "row" }, nickIn, contactIn, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
      h("div", {
        style: "font-size:11px;color:var(--text-3);line-height:1.5;margin:6px 0 10px",
        text: "Fale o apelido no lugar do nome: “manda pra Bia…”, “liga pra Bia”. Também dá para criar por voz: “quando eu falar Bia é a Beatriz Souza”.",
      }),
      list.length ? rows : h("div", { class: "empty-state", text: "Nenhum apelido ainda." }),
      watchSection,
    );
  }

  function renderClipboardBody(main: HTMLElement) {
    const list = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
    for (const item of clipboardHistory) {
      list.append(
        h(
          "div",
          { class: "list-item" },
          h("span", { style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:78%", text: item }),
          h("button", {
            class: "btn btn-ghost btn-sm",
            text: "Copiar",
            onclick: () => void navigator.clipboard?.writeText(item),
          }),
        ),
      );
    }
    main.append(
      clipboardHistory.length
        ? list
        : h("div", { class: "empty-state", text: "Histórico da área de transferência vazio." }),
    );
  }

  // ── 6. Meu PC ──────────────────────────────────────────────────────────────

  function renderPC(main: HTMLElement) {
    if (!pcStats) {
      void Bridge.jarvisPcStats().then((st) => {
        if (st) {
          pcStats = st;
          if (expanded && activeTab === "pc") render();
        }
      });
    }

    const pct = pcStats?.memPercent ?? 24;
    const r = 45;
    const circ = 2 * Math.PI * r;
    const dash = (pct / 100) * circ;

    const donutSvg = document.createElement("div");
    donutSvg.innerHTML = `<svg width="112" height="112" viewBox="0 0 112 112">
      <circle cx="56" cy="56" r="${r}" fill="none" stroke="rgba(255,255,255,0.08)" stroke-width="7"/>
      <circle cx="56" cy="56" r="${r}" fill="none" stroke="#10b981" stroke-width="7"
        stroke-linecap="round" stroke-dasharray="${dash} ${circ}" transform="rotate(-90 56 56)"/>
    </svg>`;

    const mochiCenter = createDressedMochi(40, data.mochiColor || "#f4f4f5", currentOutfit(), pcScanStep >= 0 ? "working" : "idle");

    const checkItems = [
      "Memória RAM",
      "Uso de CPU",
      "Aplicativos na inicialização",
      "Arquivos temporários",
      "Espaço em disco C:",
      "Lixeira",
    ];

    const checkList = h("div", { class: "check-list" });
    checkItems.forEach((label, idx) => {
      const done = pcScanStep > idx || pcScanStep === 99;
      checkList.append(
        h(
          "div",
          { class: done ? "done" : "" },
          h("span", { style: `color:${done ? "var(--live)" : "#6b7280"};margin-right:6px`, text: done ? "✓" : "·" }),
          h("span", { text: label }),
        ),
      );
    });

    const scanBtn = h("button", {
      class: "btn btn-sm",
      text: pcScanStep >= 0 && pcScanStep < 99 ? "Verificando…" : "Verificar PC",
      onclick: async () => {
        for (let i = 0; i <= checkItems.length; i++) {
          pcScanStep = i;
          render();
          await new Promise((res) => setTimeout(res, 280));
        }
        const fresh = await Bridge.jarvisPcStats();
        if (fresh) pcStats = fresh;
        pcScanStep = 99;
        Sound.play("finish");
        render();
      },
    });

    const statCard = (title: string, val: string, sub: string, barPct: number) =>
      h(
        "div",
        { class: "card" },
        h("div", { style: "font-size:11px;color:var(--muted)", text: title }),
        h("div", { style: "font-size:17px;font-weight:700;margin:3px 0;font-variant-numeric:tabular-nums", text: val }),
        h("div", { style: "font-size:10.5px;color:var(--muted);margin-bottom:6px", text: sub }),
        h(
          "div",
          { style: "height:3px;background:rgba(255,255,255,0.08);border-radius:99px;overflow:hidden" },
          h("div", { style: `height:100%;width:${Math.min(100, barPct)}%;background:var(--live)` }),
        ),
      );

    const quickApps: [string, string][] = [
      ["Spotify", "spotify"],
      ["Chrome", "chrome"],
      [editorName, editorName === "VS Code" ? "vscode" : editorName.toLowerCase()],
      ["Terminal", "terminal"],
      ["Explorador", "explorer"],
      ["Calculadora", "calc"],
    ];

    main.append(
      h(
        "div",
        { class: "row between" },
        h(
          "div",
          {},
          h("div", { class: "page-title", text: "Meu PC" }),
          h("div", {
            class: "page-sub",
            text: pcStats ? `Disco C: ${pcStats.diskFreeGb} GB livres · RAM ${pcStats.memUsedGb}/${pcStats.memTotalGb} GB` : "Status e atalhos rápidos",
          }),
        ),
        scanBtn,
      ),
      h(
        "div",
        { class: "card pc-hero" },
        h(
          "div",
          { class: "donut-wrap" },
          donutSvg.firstElementChild!,
          h(
            "div",
            { class: "donut-center" },
            mochiCenter,
            h("span", { style: "font-size:11.5px;font-weight:700;color:#fff;margin-top:2px", text: `${pct}%` }),
          ),
        ),
        h(
          "div",
          { style: "flex:1" },
          h("div", { style: "font-weight:600;font-size:13.5px;margin-bottom:6px", text: pcScanStep === 99 ? "Sistema verificado" : "Diagnóstico do sistema" }),
          checkList,
        ),
      ),
      h(
        "div",
        { class: "grid-3" },
        statCard("CPU", pcStats ? `${pcStats.cpuPercent}%` : "…", "Processador", pcStats?.cpuPercent ?? 0),
        statCard("Memória RAM", pcStats ? `${pcStats.memPercent}%` : "…", pcStats ? `${pcStats.memUsedGb}/${pcStats.memTotalGb} GB` : "…", pcStats?.memPercent ?? 0),
        statCard("Disco C:", pcStats ? `${pcStats.diskFreeGb} GB` : "…", pcStats ? `${pcStats.diskPercent}% usado` : "…", pcStats?.diskPercent ?? 0),
      ),
      h(
        "div",
        { class: "grid-3" },
        ...quickApps.map(([label, target]) =>
          h("button", {
            class: "card",
            style: "cursor:pointer;color:var(--text);font-weight:500;text-align:left;padding:10px 12px",
            text: label,
            onclick: () => void Bridge.jarvisOpenTarget(target),
          }),
        ),
      ),
    );
  }

  // ── 7. Ajustes ─────────────────────────────────────────────────────────────

  let monitors: { key: string; label: string }[] | null = null;

  /** Ajustes → Notch: when it shows, and on which display. */
  function renderNotchSettings(main: HTMLElement) {
    const mode = State.settings.jarvisNotchMode ?? "always";
    const option = (id: "always" | "hidden", title: string, desc: string) =>
      h(
        "button",
        {
          class: `brain-card ${mode === id ? "selected" : ""}`,
          style: "text-align:left",
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisNotchMode: id });
            render();
          },
        },
        h("div", { style: "font-weight:600;font-size:12.5px;margin-bottom:4px", text: title }),
        h("div", { style: "font-size:11.5px;color:var(--text-3);line-height:1.4", text: desc }),
      );

    if (monitors == null) {
      void Bridge.listMonitors().then((list) => {
        monitors = list ?? [];
        if (expanded && activeTab === "settings" && settingsSubTab === "notch") render();
      });
    }
    const screen = State.settings.screen || "primary";
    const select = h(
      "select",
      { class: "input", style: "width:100%;height:34px;color-scheme:dark" },
      h("option", { value: "primary", text: "Monitor principal" }),
      h("option", { value: "cursor", text: "Os dois — aparece no monitor onde o mouse estiver" }),
      ...(monitors ?? []).map((m) => h("option", { value: m.key, text: m.label })),
    ) as HTMLSelectElement;
    select.value = [...select.options].some((o) => o.value === screen) ? screen : "primary";
    select.addEventListener("change", async () => {
      await saveSettings({ ...State.settings, screen: select.value });
      render();
    });

    const theme = State.settings.jarvisTheme || "graphite";
    const shape = State.settings.jarvisNotchShape || "notch";
    const shapeOption = (id: string, title: string, desc: string) =>
      h(
        "button",
        {
          class: `brain-card ${shape === id ? "selected" : ""}`,
          style: "text-align:left",
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisNotchShape: id });
            render();
          },
        },
        h("div", { class: `shape-swatch ${id}` }),
        h("div", { style: "font-weight:600;font-size:12.5px;margin-bottom:4px", text: title }),
        h("div", { style: "font-size:11.5px;color:var(--text-3);line-height:1.4", text: desc }),
      );
    const themeOption = (id: string, title: string, desc: string) =>
      h(
        "button",
        {
          class: `brain-card ${theme === id ? "selected" : ""}`,
          style: "text-align:left",
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisTheme: id });
            render();
          },
        },
        h("div", { class: `theme-swatch ${id}` }),
        h("div", { style: "font-weight:600;font-size:12.5px;margin-bottom:4px", text: title }),
        h("div", { style: "font-size:11.5px;color:var(--text-3);line-height:1.4", text: desc }),
      );

    main.append(
      sectionHead("Visual"),
      h(
        "div",
        { style: "display:grid;grid-template-columns:repeat(3,1fr);gap:10px" },
        themeOption("graphite", "Grafite", "Sóbrio e plano, cinza neutro. O visual clássico do Jarvis."),
        themeOption("glass", "Cristal", "Vidro com brilho nas bordas e luz suave colorida, no estilo do iOS."),
        themeOption("midnight", "Meia-noite", "O mesmo vidro em tons de azul profundo."),
        themeOption("aurora", "Aurora", "Vidro com luz violeta e rosa."),
        themeOption("emerald", "Esmeralda", "Vidro com luz verde e turquesa."),
      ),
      sectionHead("Formato do notch"),
      h(
        "div",
        { style: "display:grid;grid-template-columns:repeat(3,1fr);gap:10px" },
        shapeOption("notch", "Notch", "Preso à borda de cima, cantos de baixo arredondados."),
        shapeOption("island", "Ilha", "Uma cápsula flutuando logo abaixo da borda, como a Ilha Dinâmica."),
        shapeOption("tab", "Aba", "Preso à borda, com cantos quase retos."),
      ),
      sectionHead("Quando o notch aparece"),
      h(
        "div",
        { class: "grid-2" },
        option("always", "Sempre visível", "A pílula fica no topo da tela o tempo todo, com o relógio e a música tocando."),
        option("hidden", "Escondido", "Some do topo. Aparece quando você passa o mouse no meio da borda de cima ou quando fala com o Jarvis."),
      ),
      sectionHead("Em qual monitor"),
      select,
      h("div", {
        style: "font-size:11.5px;color:var(--text-3);line-height:1.5;margin-top:6px",
        text: "“Os dois” leva o notch para o monitor em que o mouse está; ele muda de monitor quando você move o mouse (com o painel fechado).",
      }),
      sectionHead("Informações no notch"),
      renderNotchInfoSettings(),
      sectionHead("Fechar o painel"),
      h("div", {
        style: "font-size:12px;color:var(--text-2);line-height:1.5",
        text: "Clique fora do painel, aperte Esc ou use a seta no canto da barra lateral.",
      }),
    );
  }

  /** Top of Ajustes when a newer version exists: notes, download, progress. */
  function renderUpdateCard(): HTMLElement | string {
    const u = updateInfo;
    if (!u?.version) return "";
    const busy = updateState.phase === "downloading" || updateState.phase === "installing";
    const install = async () => {
      updateState = { phase: "downloading", pct: 0, note: "" };
      render();
      try {
        await Bridge.updateInstall();
      } catch (e) {
        updateState = { phase: "error", pct: 0, note: String(e).replace(/^Error:\s*/, "") };
        render();
      }
    };
    const notes = u.notes
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.startsWith("- "))
      .slice(0, 5)
      .map((l) => l.slice(2).replace(/\*\*/g, ""));
    return h(
      "div",
      { class: "update-card" },
      h(
        "div",
        { class: "update-head" },
        h("div", {}, h("div", { class: "update-title", text: `Nova versão: Jarvis ${u.version}` }), h("div", { class: "tt-hint", style: "margin:2px 0 0", text: `Você está na ${u.current}.` })),
        h("button", { class: "btn", text: busy ? "Atualizando…" : "Baixar e instalar", onclick: () => !busy && void install() }),
      ),
      notes.length ? h("ul", { class: "update-notes" }, ...notes.map((n) => h("li", { text: n }))) : "",
      busy ? h("div", { class: "mic-meter", style: "margin-top:12px" }, h("div", { class: "mic-fill over", style: `width:${updateState.pct}%` })) : "",
      updateState.note ? h("div", { class: `tt-note ${updateState.phase === "error" ? "err" : ""}`, style: "padding:8px 0 0", text: updateState.note }) : "",
      h("div", { class: "tt-hint", style: "margin:8px 0 0", text: "A atualização é baixada do GitHub do projeto, conferida pela assinatura e instalada. O Windows pode pedir permissão de administrador." }),
    );
  }

  /** Current version and "check now" (IA & Voz → Sobre). */
  function renderAboutUpdates(): HTMLElement {
    const line = h("span", {
      class: "tt-row-meta",
      text:
        updateState.phase === "checking"
          ? "Verificando…"
          : updateInfo?.version
            ? `Nova versão ${updateInfo.version} disponível (no topo de Ajustes)`
            : updateState.phase === "latest"
              ? "Você está na versão mais recente."
              : updateState.phase === "error"
                ? updateState.note
                : "",
    });
    const check = async () => {
      updateState = { phase: "checking", pct: 0, note: "" };
      render();
      try {
        const u = await Bridge.updateCheck();
        updateInfo = u;
        updateState = { phase: u.version ? "idle" : "latest", pct: 0, note: "" };
      } catch (e) {
        updateState = { phase: "error", pct: 0, note: String(e).replace(/^Error:\s*/, "") };
      }
      render();
    };
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: `Jarvis ${updateInfo?.current ?? ""}`.trim() }),
      h("div", { class: "tt-volume" }, line, h("button", { class: "btn btn-sm", text: "Verificar atualizações", onclick: () => void check() })),
    );
  }

  /** Voice enrollment: the user says "Jarvis" a few times; afterwards the name
   *  is recognized on the PC (instant, offline) instead of in the cloud. */
  function renderWakeEnroll(): HTMLElement {
    const st = wakeStatus;
    const wanted = st?.samplesWanted ?? 6;
    const local = (State.settings.jarvisWakeEngine ?? "local") === "local";

    const startEnroll = async () => {
      // A fresh enrollment: the old model goes, samples are recorded one by one.
      // The cloud listener must not take the samples for calls meanwhile.
      Voice.setLocalWake(true);
      await Bridge.wakewordReset();
      enroll = { index: 0, note: "", err: false, busy: false };
      render();
      void nextSample();
    };
    const nextSample = async () => {
      if (!enroll) return;
      if (enroll.index >= wanted) {
        enroll = { ...enroll, busy: true, note: "Criando o modelo da sua voz…", err: false };
        render();
        try {
          wakeStatus = await Bridge.wakewordBuild();
          enroll = null;
          Voice.setLocalWake(wakeStatus.active);
          showToast("Pronto! Agora o Jarvis reconhece o seu “Jarvis” no próprio PC.", 7000);
          refreshWakeSoon();
        } catch (e) {
          if (enroll) enroll = { ...enroll, busy: false, note: String(e).replace(/^Error:\s*/, ""), err: true };
        }
        render();
        return;
      }
      enroll = { ...enroll, busy: true, note: "", err: false };
      render();
      try {
        await Bridge.wakewordRecordSample(enroll.index + 1);
        if (!enroll) return; // cancelled meanwhile
        enroll = { ...enroll, index: enroll.index + 1, busy: false, note: "", err: false };
        Sound.play("pop");
        render();
        window.setTimeout(() => void nextSample(), 700);
      } catch (e) {
        if (!enroll) return;
        enroll = { ...enroll, busy: false, note: String(e).replace(/^Error:\s*/, ""), err: true };
        render();
        window.setTimeout(() => void nextSample(), 1600);
      }
    };

    const card = h("div", { class: "tt-card", style: "margin-bottom:8px" });
    if (enroll) {
      const dots = h(
        "div",
        { class: "enroll-dots" },
        ...Array.from({ length: wanted }, (_, i) => h("span", { class: i < enroll!.index ? "done" : i === enroll!.index ? "now" : "" })),
      );
      card.append(
        h("div", { class: "enroll-say", text: enroll.index >= wanted ? "Quase lá…" : enroll.busy ? "Diga “Jarvis”" : "Prepare-se…" }),
        h("div", { class: "tt-hint", style: "text-align:center;margin:4px 0 12px", text: enroll.index >= wanted ? "" : `Amostra ${enroll.index + 1} de ${wanted} — do jeito que você chama normalmente, a uma distância normal do microfone.` }),
        dots,
        enroll.note ? h("div", { class: `tt-note ${enroll.err ? "err" : ""}`, style: "text-align:center", text: enroll.note }) : "",
        h("div", { style: "text-align:center;margin-top:10px" }, h("button", { class: "btn btn-sm btn-ghost", text: "Cancelar", onclick: () => ((enroll = null), refreshWakeSoon(), render()) })),
      );
      return card;
    }

    const enrolled = Boolean(st?.enrolled);
    card.append(
      h("div", { class: "tt-card-title", text: enrolled ? (st?.active ? "Ativo — reconhecido no seu PC" : "Cadastrado (desligado)") : "Sem cadastro — usando a nuvem" }),
      h("div", {
        class: "tt-hint",
        style: "margin:0 0 10px",
        text: enrolled
          ? "O Jarvis compara o que o microfone ouve com o seu “Jarvis”, no próprio PC: responde na hora, sem internet, e a conversa do ambiente não é enviada para lugar nenhum."
          : "Diga “Jarvis” 6 vezes e o Jarvis passa a reconhecer o seu chamado no próprio PC — mais rápido, sem gastar cota e muito melhor com barulho.",
      }),
      st?.error ? h("div", { class: "tt-note err", style: "padding:4px 0 10px", text: `Microfone: ${st.error}` }) : "",
      h(
        "div",
        { class: "tt-actions" },
        h("button", { class: "btn btn-sm", text: enrolled ? "Refazer cadastro" : "Cadastrar minha voz", onclick: () => void startEnroll() }),
        enrolled
          ? h("button", {
              class: "btn btn-sm btn-ghost",
              text: "Remover",
              onclick: async () => {
                wakeStatus = await Bridge.wakewordReset();
                Voice.setLocalWake(false);
                render();
              },
            })
          : "",
      ),
    );
    if (enrolled) {
      const th = State.settings.jarvisWakeThreshold ?? 0.45;
      const slider = h("input", { type: "range", min: "0.35", max: "0.7", step: "0.05", value: String(th), class: "tt-range" }) as HTMLInputElement;
      slider.addEventListener("change", async () => {
        await saveSettings({ ...State.settings, jarvisWakeThreshold: Number(slider.value) });
        refreshWakeSoon();
      });
      const engine = h("button", {
        class: `btn btn-sm ${local ? "" : "btn-ghost"}`,
        text: local ? "Usando o reconhecimento local" : "Usando a nuvem — trocar para local",
        onclick: async () => {
          await saveSettings({ ...State.settings, jarvisWakeEngine: local ? "cloud" : "local" });
          refreshWakeSoon();
          render();
        },
      });
      card.append(
        h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Sensibilidade do chamado" }),
        h("div", { class: "tt-volume" }, h("span", { text: "Mais fácil" }), slider, h("span", { text: "Mais rigoroso" })),
        h("div", { class: "tt-hint", style: "margin:4px 0 0", text: "Se ele não acorda, puxe para “mais fácil”. Se acorda sozinho, para “mais rigoroso”." }),
        h("div", { style: "margin-top:12px" }, engine),
      );
    }
    return card;
  }

  /** Which microphone the Jarvis listens to, how sensitive, and a live meter. */
  function renderMicSettings(): HTMLElement {
    const current = State.settings.jarvisMicDevice ?? "";
    const select = h(
      "select",
      { class: "input", style: "width:100%;height:34px;color-scheme:dark" },
      h("option", { value: "", text: "Padrão do Windows para apps" }),
      h("option", { value: "communications", text: "Fone/headset — dispositivo de comunicação do Windows" }),
    ) as HTMLSelectElement;
    select.value = current;
    // Real device names (available once the microphone was allowed).
    void navigator.mediaDevices?.enumerateDevices().then((devices) => {
      for (const d of devices) {
        if (d.kind !== "audioinput" || d.deviceId === "default" || d.deviceId === "communications") continue;
        select.append(h("option", { value: d.deviceId, text: d.label || "Microfone" }));
      }
      select.value = current;
      if (select.value !== current) select.value = "";
    });
    select.addEventListener("change", async () => {
      const name = select.value && select.value !== "communications" ? select.selectedOptions[0]?.text ?? "" : "";
      await saveSettings({ ...State.settings, jarvisMicDevice: select.value, jarvisMicName: name });
      Voice.restart();
      refreshWakeSoon();
    });

    const sens = State.settings.jarvisMicSensitivity ?? "alta";
    const chips = h(
      "div",
      { class: "tt-chips", style: "margin:10px 0 0" },
      ...([
        ["alta", "Alta — voz normal"],
        ["media", "Média"],
        ["baixa", "Baixa — lugar barulhento"],
      ] as const).map(([id, label]) =>
        h("button", {
          class: `tt-chip ${sens === id ? "on" : ""}`,
          text: label,
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisMicSensitivity: id });
            Voice.restart();
            refreshWakeSoon();
            render();
          },
        }),
      ),
    );

    // Live meter: the bar is your voice, the line is how loud it must be.
    const fill = h("div", { class: "mic-fill" });
    const mark = h("div", { class: "mic-mark" });
    const meter = h("div", { class: "mic-meter" }, fill, mark);
    const hint = h("div", { class: "tt-hint", style: "margin:6px 0 0", text: "Fale normalmente: a barra deve passar da linha. Se só passa gritando, escolha outro microfone ou a sensibilidade Alta." });
    let smooth = 0;
    Voice.onLevel = (level, threshold) => {
      if (!meter.isConnected) return;
      smooth = Math.max(level, smooth * 0.85);
      // A log scale reads better than a linear one for voices.
      const pos = (v: number) => Math.max(0, Math.min(100, ((Math.log10(Math.max(v, 0.001)) + 3) / 2.3) * 100));
      fill.style.width = `${pos(smooth)}%`;
      fill.classList.toggle("over", smooth > threshold);
      mark.style.left = `${pos(threshold)}%`;
    };

    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Microfone do Jarvis" }),
      select,
      h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Sensibilidade" }),
      chips,
      h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Nível agora" }),
      meter,
      hint,
      renderVocabField(),
    );
  }

  /** Which voice the Jarvis speaks with, and how fast. */
  let ttsVoices: { id: string; label: string }[] | null = null;
  function renderVoicePicker(): HTMLElement {
    const voice = State.settings.jarvisTtsVoice || "pt-BR-AntonioNeural";
    const rate = State.settings.jarvisTtsRate ?? 8;
    const select = h("select", { class: "input", style: "width:100%;height:34px;color-scheme:dark" }) as HTMLSelectElement;
    const fill = (list: { id: string; label: string }[]) => {
      select.replaceChildren(
        ...list.map((v) => h("option", { value: v.id, text: v.label }) as HTMLOptionElement),
        h("option", { value: "windows", text: "Voz do Windows (offline, mais robótica)" }) as HTMLOptionElement,
      );
      select.value = [...select.options].some((o) => o.value === voice) ? voice : "pt-BR-AntonioNeural";
    };
    if (ttsVoices) fill(ttsVoices);
    else {
      select.append(h("option", { value: voice, text: "Carregando vozes…" }) as HTMLOptionElement);
      void Bridge.ttsVoices().then((list) => {
        ttsVoices = list ?? [];
        fill(ttsVoices);
      });
    }
    const status = h("span", { class: "tt-row-meta" });
    const sample = async () => {
      status.textContent = "Falando…";
      await Voice.speak("Olá! Esta é a minha voz. Em que posso te ajudar hoje?");
      status.textContent = "";
    };
    select.addEventListener("change", async () => {
      await saveSettings({ ...State.settings, jarvisTtsVoice: select.value });
      void sample();
    });
    const speeds: [number, string][] = [
      [-10, "Calma"],
      [0, "Normal"],
      [8, "Ágil"],
      [20, "Rápida"],
    ];
    const chips = h(
      "div",
      { class: "tt-chips" },
      ...speeds.map(([v, label]) =>
        h("button", {
          class: `tt-chip ${rate === v ? "on" : ""}`,
          text: label,
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisTtsRate: v });
            render();
            void sample();
          },
        }),
      ),
    );
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Voz" }),
      select,
      h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Velocidade" }),
      chips,
      h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Som quando ele começa a ouvir" }),
      h(
        "div",
        { class: "tt-chips" },
        ...(
          [
            ["loud", "Alto"],
            ["soft", "Suave"],
            ["classic", "Clássico"],
            ["off", "Desligado"],
          ] as const
        ).map(([v, label]) =>
          h("button", {
            class: `tt-chip ${(State.settings.jarvisListenChime || "loud") === v ? "on" : ""}`,
            text: label,
            onclick: async () => {
              await saveSettings({ ...State.settings, jarvisListenChime: v });
              listenCue();
              render();
            },
          }),
        ),
      ),
      h(
        "div",
        { class: "tt-volume", style: "margin-top:10px" },
        h("span", {
          class: "tt-hint",
          style: "margin:0",
          text: "Vozes neurais da Microsoft, as mesmas do \"Ler em voz alta\" do Edge — naturais em qualquer PC, sem instalar nada. Precisam de internet.",
        }),
        h("button", { class: "btn btn-sm", text: "Ouvir", onclick: () => void sample() }),
      ),
      status,
    );
  }

  /** Names, games and apps the transcription should spell right. */
  function renderVocabField(): HTMLElement {
    const input = h("input", { class: "input", placeholder: "Ex.: FC 26, Daniel Levy, Valorant", style: "flex:1;height:34px" }) as HTMLInputElement;
    input.value = State.settings.jarvisVoiceVocab ?? "";
    const status = h("span", { class: "tt-row-meta" });
    const save = async () => {
      await saveSettings({ ...State.settings, jarvisVoiceVocab: input.value.trim() });
      status.textContent = "Salvo";
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void save();
    });
    return h(
      "div",
      {},
      h("div", { class: "tt-card-title", style: "margin-top:14px", text: "Palavras que o Jarvis deve conhecer" }),
      h("div", { class: "tt-searchbar" }, input, h("button", { class: "btn", text: "Salvar", onclick: () => void save() }), status),
      h("div", {
        class: "tt-hint",
        style: "margin:6px 0 0",
        text: "Nomes de pessoas, jogos e apps que ele costuma entender errado, separados por vírgula. Apps comuns (Spotify, WhatsApp, Claude…) já estão incluídos.",
      }),
    );
  }

  /** The OpenBot webhook secret: written to the Windows vault, never shown back. */
  function secretField(): HTMLElement {
    const input = h("input", { class: "input", type: "password", placeholder: "Segredo HMAC do webhook", style: "flex:1;height:34px" }) as HTMLInputElement;
    const status = h("span", { class: "tt-row-meta" });
    void Bridge.secretPresent("openbot-webhook-secret").then((has) => {
      if (has && !status.textContent) status.textContent = "Configurado";
    });
    const save = async () => {
      if (!input.value.trim()) return;
      await Bridge.secretSet("openbot-webhook-secret", input.value.trim());
      input.value = "";
      status.textContent = "Salvo no cofre do Windows";
    };
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Segredo do webhook do OpenBot (opcional)" }),
      h("div", { class: "tt-searchbar" }, input, h("button", { class: "btn", text: "Salvar", onclick: () => void save() }), status),
      h("div", { class: "tt-hint", style: "margin:6px 0 0", text: "Fica guardado no Cofre de Credenciais do Windows; o Jarvis nunca mostra de volta." }),
    );
  }

  /** Backup: routines, nicknames, watched contacts, voice and looks, in a file. */
  function renderBackup(): HTMLElement {
    const status = h("div", { class: "tt-hint", style: "margin:6px 0 0" });
    const picker = h("input", { type: "file", accept: ".json,application/json", style: "display:none" }) as HTMLInputElement;
    picker.addEventListener("change", async () => {
      const file = picker.files?.[0];
      picker.value = "";
      if (!file) return;
      try {
        const res = await Bridge.configImport(await file.text());
        await saveSettings({ ...State.settings, ...(res.settings as Partial<Settings>) });
        const fresh = await Bridge.jarvisDataLoad();
        if (fresh) data = { ...data, ...fresh };
        showToast(res.message);
        render();
      } catch (e) {
        status.textContent = String(e).replace(/^Error:\s*/, "");
      }
    });
    const exportIt = async () => {
      try {
        const path = await Bridge.configExport();
        status.textContent = `Salvo em ${path}`;
        void Bridge.jarvisOpenTarget(path.replace(/[\\/][^\\/]*$/, ""));
      } catch (e) {
        status.textContent = String(e).replace(/^Error:\s*/, "");
      }
    };
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Backup das configurações" }),
      h(
        "div",
        { class: "row", style: "gap:8px" },
        h("button", { class: "btn", text: "Exportar", onclick: () => void exportIt() }),
        h("button", { class: "btn btn-ghost", text: "Importar…", onclick: () => picker.click() }),
        picker,
      ),
      status,
      h("div", {
        class: "tt-hint",
        style: "margin:6px 0 0",
        text: "Rotinas, apelidos, contatos com aviso, voz, visual e preferências num arquivo (em Documentos) — para levar a outro PC ou guardar. Senhas e chaves não vão junto: elas ficam no cofre do Windows.",
      }),
    );
  }

  /** The Jarvis browser keeps its own logins: sign in there once per site. */
  function renderBrowserLogins(): HTMLElement {
    const status = h("div", { class: "tt-hint", style: "margin:6px 0 0" });
    const go = (url: string) => async () => {
      status.textContent = "Abrindo…";
      status.textContent = await Bridge.browserLoginOpen(url).catch((e) => String(e).replace(/^Error:\s*/, ""));
    };
    const site = (label: string, url: string) => h("button", { class: "btn btn-ghost btn-sm", text: label, onclick: () => void go(url)() });
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Logins do navegador do Jarvis" }),
      h(
        "div",
        { class: "row", style: "gap:6px;flex-wrap:wrap" },
        site("Google / YouTube", "https://accounts.google.com"),
        site("Instagram", "https://www.instagram.com/accounts/login/"),
        site("Facebook", "https://www.facebook.com/login"),
        site("X (Twitter)", "https://x.com/i/flow/login"),
        site("Outro site…", "https://www.google.com"),
      ),
      status,
      h("div", {
        class: "tt-hint",
        style: "margin:6px 0 0",
        text: "Quando o Jarvis precisa mexer num site (buscar, clicar, preencher), ele usa um navegador próprio — o Chrome não deixa controlar o seu perfil principal. Entre nas suas contas nele uma vez: o login fica salvo. Para só abrir um site (“abre o YouTube”), ele usa o seu navegador normal, já logado.",
      }),
    );
  }

  /** OBS by voice: its WebSocket password in the Windows vault, and a test. */
  function renderObsSettings(): HTMLElement {
    const input = h("input", { class: "input", type: "password", placeholder: "Senha do servidor WebSocket do OBS", style: "flex:1;height:34px" }) as HTMLInputElement;
    const status = h("span", { class: "tt-row-meta" });
    void Bridge.secretPresent("obs-password").then((has) => {
      if (has && !status.textContent) status.textContent = "Senha salva";
    });
    const save = async () => {
      if (!input.value.trim()) return;
      await Bridge.secretSet("obs-password", input.value.trim());
      input.value = "";
      status.textContent = "Senha salva";
    };
    const test = async () => {
      status.textContent = "Testando…";
      status.textContent = await Bridge.obsTest().catch((e) => String(e).replace(/^Error:\s*/, ""));
    };
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "OBS Studio" }),
      h(
        "div",
        { class: "tt-searchbar" },
        input,
        h("button", { class: "btn", text: "Salvar", onclick: () => void save() }),
        h("button", { class: "btn btn-ghost", text: "Testar", onclick: () => void test() }),
      ),
      status,
      h("div", {
        class: "tt-hint",
        style: "margin:6px 0 0",
        text: "No OBS: Ferramentas → Configurações do servidor WebSocket → Ativar servidor (porta 4455) e copie a senha. Depois: “começa a gravar”, “encerra a live”, “troca para a cena Câmera”, “muta o microfone do OBS”.",
      }),
    );
  }

  /** The code editor projects open in, among the ones found on this PC. */
  let editorsFound: { name: string; path: string }[] | null = null;
  function renderEditorPicker(): HTMLElement {
    const select = h("select", { class: "input", style: "width:100%;height:34px;color-scheme:dark" }) as HTMLSelectElement;
    const status = h("span", { class: "tt-row-meta" });
    const fill = (list: { name: string; path: string }[]) => {
      select.replaceChildren();
      const auto = list[0]?.name;
      select.append(h("option", { value: "", text: auto ? `Automático (${auto})` : "Nenhum editor encontrado" }) as HTMLOptionElement);
      for (const e of list) select.append(h("option", { value: e.path, text: e.name, title: e.path }) as HTMLOptionElement);
      const saved = State.settings.jarvisCodeEditor ?? "";
      // A saved editor that was uninstalled falls back to automatic.
      select.value = list.some((e) => e.path.toLowerCase() === saved.toLowerCase()) ? list.find((e) => e.path.toLowerCase() === saved.toLowerCase())!.path : "";
    };
    if (editorsFound) fill(editorsFound);
    else {
      select.append(h("option", { value: "", text: "Procurando editores instalados…" }) as HTMLOptionElement);
      void Bridge.listCodeEditors().then((list) => {
        editorsFound = list ?? [];
        fill(editorsFound);
      });
    }
    select.addEventListener("change", async () => {
      await saveSettings({ ...State.settings, jarvisCodeEditor: select.value });
      editorName = (await Bridge.codeEditorName()) || editorName;
      status.textContent = `Projetos abrem no ${editorName}`;
    });
    return h(
      "div",
      { class: "tt-card", style: "margin-bottom:8px" },
      h("div", { class: "tt-card-title", text: "Editor de código dos projetos" }),
      select,
      h(
        "div",
        { class: "tt-volume", style: "margin-top:6px" },
        h("span", { class: "tt-hint", style: "margin:0", text: "Usado em \"Abrir no…\" dos projetos e quando você pede \"abre o projeto X\". A lista mostra só os editores instalados neste PC." }),
        status,
      ),
    );
  }

  /** Obsidian, projects folder and the OpenBot webhook (empty = automatic). */
  function renderFolderSettings(): HTMLElement {
    type Field = "jarvisObsidianVault" | "jarvisProjectsDir" | "jarvisOpenbotWebhookUrl";
    const field = (key: Field, title: string, placeholder: string, hint: string) => {
      const input = h("input", { class: "input", placeholder, style: "flex:1;height:34px" }) as HTMLInputElement;
      input.value = (State.settings[key] as string | undefined) ?? "";
      const status = h("span", { class: "tt-row-meta" });
      const save = async () => {
        await saveSettings({ ...State.settings, [key]: input.value.trim() });
        status.textContent = "Salvo";
      };
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") void save();
      });
      return h(
        "div",
        { class: "tt-card", style: "margin-bottom:8px" },
        h("div", { class: "tt-card-title", text: title }),
        h("div", { class: "tt-searchbar" }, input, h("button", { class: "btn", text: "Salvar", onclick: () => void save() }), status),
        h("div", { class: "tt-hint", style: "margin:6px 0 0", text: hint }),
      );
    };
    return h(
      "div",
      {},
      field("jarvisObsidianVault", "Vault do Obsidian", "Automático: Documentos\\Obsidian Vault\\Memoria-Unificada", "Pasta das suas notas (aba Notas e memória do Jarvis). Vazio = automático."),
      field("jarvisProjectsDir", "Pasta de projetos", "Automático: D:\\Projetos ou Usuário\\Projetos", "Onde o Jarvis procura seus projetos (aba Projetos e busca de arquivos)."),
      field("jarvisOpenbotWebhookUrl", "Webhook do OpenBot (opcional)", "https://…", "Para delegar tarefas e mandar WhatsApp pelo seu OpenBot."),
      secretField(),
    );
  }

  /** Weather, PC usage rings and FPS: what the pill shows next to the clock. */
  function renderNotchInfoSettings(): HTMLElement {
    type Flag = "jarvisNotchWeather" | "jarvisNotchPc" | "jarvisNotchFps" | "jarvisNotchNet" | "jarvisGameMode" | "jarvisMorningBriefing" | "jarvisGameOtherScreen" | "jarvisAutoDnd";
    const row = (key: Flag, title: string, desc: string, extra?: HTMLElement) => {
      const on = key === "jarvisGameOtherScreen" || key === "jarvisAutoDnd" ? State.settings[key] !== false : Boolean(State.settings[key]);
      return h(
        "div",
        { class: `nw-setting ${on ? "on" : ""}` },
        h(
          "button",
          {
            class: "nw-setting-head",
            role: "switch",
            "aria-checked": String(on),
            onclick: async () => {
              await saveSettings({ ...State.settings, [key]: !on });
              render();
            },
          },
          h(
            "div",
            { class: "nw-setting-text" },
            h("div", { class: "nw-setting-title", text: title }),
            h("div", { class: "nw-setting-desc", text: desc }),
          ),
          h("span", { class: "nw-switch" }, h("span", { class: "nw-knob" })),
        ),
        on && extra ? extra : "",
      );
    };

    // City + a "Testar" that shows what the notch will show.
    const city = h("input", {
      class: "input",
      placeholder: "Sua cidade — ex.: Recife, Campinas, Lisboa",
      style: "flex:1;height:34px",
    }) as HTMLInputElement;
    city.value = State.settings.jarvisWeatherCity ?? "";
    const result = h("div", { class: "nw-setting-result" });
    const note = (text: string, err = false) => {
      weatherNote = { text, err };
      result.textContent = text;
      result.className = `nw-setting-result${err ? " err" : ""}`;
    };
    if (weatherNote) note(weatherNote.text, weatherNote.err);
    const test = async () => {
      const name = city.value.trim();
      if (!name) return note("Digite o nome da sua cidade.", true);
      note("Buscando…");
      try {
        const w = await widgets.testCity(name);
        note(`${w.place}: ${Math.round(w.temp)}° · ${w.label} · chance de chuva hoje ${w.rainChance}%`);
        if (name !== (State.settings.jarvisWeatherCity ?? "")) {
          await saveSettings({ ...State.settings, jarvisWeatherCity: name });
          render();
        }
      } catch (e) {
        note(String(e), true);
      }
    };
    city.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void test();
    });
    const weatherExtra = h(
      "div",
      { class: "nw-setting-extra" },
      h("div", { style: "display:flex;gap:8px" }, city, h("button", { class: "btn", text: "Salvar", onclick: () => void test() })),
      result,
    );
    if (!weatherNote && State.settings.jarvisNotchWeather && !(State.settings.jarvisWeatherCity ?? "").trim()) {
      result.textContent = "Defina a cidade para o clima aparecer.";
    }

    const fpsExtra = h("div", {
      class: "nw-setting-extra nw-setting-note",
      text:
        "Aparece sozinho quando um jogo (ou programa que use muito a placa de vídeo) está em primeiro plano. Usa os contadores DirectX do Windows, como o PresentMon: precisa do OpenJarvis aberto como administrador. Jogos em Vulkan/OpenGL podem não aparecer.",
    });

    if (State.settings.jarvisNotchFps && widgets.fpsAvailable() === false) {
      fpsExtra.append(
        h("div", {
          class: "nw-setting-result err",
          text: "Agora o FPS não pode ser medido: abra o OpenJarvis como administrador.",
        }),
      );
    }

    return h(
      "div",
      { class: "nw-settings" },
      row("jarvisNotchWeather", "Clima", "Temperatura e se vai fazer sol ou chuva na sua cidade.", weatherExtra),
      row("jarvisNotchPc", "Uso do PC", "Círculos de CPU, RAM, placa de vídeo e disco. Passe o mouse para ver o componente."),
      row("jarvisNotchFps", "Medidor de FPS", "Quadros por segundo do jogo aberto.", fpsExtra),
      row(
        "jarvisGameOtherScreen",
        "Jogo em tela cheia → outro monitor",
        "Jogos em tela cheia exclusiva cobrem tudo, até o notch. Com dois monitores, o notch (com o FPS) vai para o outro monitor durante o jogo e volta quando você sai. Dica: com “Janela sem bordas” no jogo, o notch fica por cima no mesmo monitor.",
      ),
      row("jarvisNotchNet", "Internet", "Ping ao lado do relógio; avisa em vermelho quando a conexão cai."),
      row("jarvisGameMode", "Modo jogo", "Com um jogo aberto, o notch mostra só o FPS e o Jarvis fica sem sons. No fim, um resumo da sessão (tempo, FPS médio e 1% low)."),
      row("jarvisAutoDnd", "Não perturbe automático", "Em tela cheia (jogo, vídeo) ou com o OBS gravando/ao vivo, o Jarvis guarda os avisos do WhatsApp e os lembretes e conta tudo quando acabar."),
      row("jarvisMorningBriefing", "Bom dia", "No primeiro início do dia, o Jarvis fala o clima, seus compromissos e tarefas. Também dá para pedir: “bom dia, Jarvis”."),
    );
  }

  function renderSettings(main: HTMLElement) {
    const subNav = h(
      "div",
      { class: "row", style: "gap:5px" },
      h("button", {
        class: `filter-chip ${settingsSubTab === "brain" ? "on" : ""}`,
        text: "IA & Voz",
        onclick: () => { settingsSubTab = "brain"; render(); },
      }),
      h("button", {
        class: `filter-chip ${settingsSubTab === "notch" ? "on" : ""}`,
        text: "Notch",
        onclick: () => { settingsSubTab = "notch"; render(); },
      }),
      h("button", {
        class: `filter-chip ${settingsSubTab === "customize" ? "on" : ""}`,
        text: "Mascote",
        onclick: () => { settingsSubTab = "customize"; render(); },
      }),
    );

    if (settingsSubTab === "notch") {
      main.append(h("div", { class: "row between" }, h("div", { class: "page-title", text: "Ajustes" }), subNav), renderUpdateCard());
      renderNotchSettings(main);
      return;
    }

    if (settingsSubTab === "customize") {
      const previewMochi = createDressedMochi(80, data.mochiColor || "#f4f4f5", currentOutfit(), "idle");
      const nameIn = h("input", { class: "input", value: data.userName, style: "width:130px" }) as HTMLInputElement;
      const assistIn = h("input", { class: "input", value: data.assistantName || "Jarvis", style: "width:130px" }) as HTMLInputElement;
      const mochiColors = ["#f4f4f5", "#a855f7", "#38bdf8", "#4ade80", "#fb923c", "#f43f5e", "#facc15", "#1f2937"];
      const color = (data.mochiColor || "#f4f4f5").toLowerCase();
      const finish = mochiFinish;

      const setColor = async (c: string) => {
        data.mochiColor = c;
        await saveData();
        render();
      };
      // Any colour: the system picker; applied while dragging, saved on release.
      const picker = h("input", { type: "color", class: "mascot-picker", value: color, title: "Escolher qualquer cor" }) as HTMLInputElement;
      picker.addEventListener("input", () => {
        data.mochiColor = picker.value;
      });
      picker.addEventListener("change", () => void setColor(picker.value));
      const hexIn = h("input", { class: "input mascot-hex", value: color, spellcheck: "false", maxlength: 7 }) as HTMLInputElement;
      hexIn.addEventListener("change", () => {
        const v = hexIn.value.trim().replace(/^([^#])/, "#$1");
        if (/^#[0-9a-f]{6}$/i.test(v)) void setColor(v.toLowerCase());
        else hexIn.value = color;
      });

      main.append(
        h("div", { class: "row between" }, h("div", { class: "page-title", text: "Ajustes" }), subNav), renderUpdateCard(),
        h(
          "div",
          { class: "mascot-grid" },
          h(
            "div",
            { class: "card mascot-wardrobe" },
            h("div", { class: "mascot-preview" }, previewMochi, h("div", { class: "mascot-name", text: data.assistantName || "Jarvis" })),
            h(
              "div",
              { class: "mascot-groups" },
              ...OUTFIT_GROUPS.map((g) =>
                h(
                  "div",
                  { class: "mascot-group" },
                  h("div", { class: "mascot-group-title", text: g.title }),
                  h(
                    "div",
                    { class: "row", style: "flex-wrap:wrap;gap:5px" },
                    ...g.items.map((o) =>
                      h("button", {
                        class: `outfit-chip ${currentOutfit() === o.id ? "on" : ""}`,
                        text: o.label,
                        onclick: async () => {
                          data.mochiOutfit = o.id;
                          await saveData();
                          render();
                        },
                      }),
                    ),
                  ),
                ),
              ),
            ),
          ),
          h(
            "div",
            { class: "card", style: "display:flex;flex-direction:column;gap:14px" },
            h(
              "div",
              {},
              h("div", { class: "mascot-label", text: "Cor do mascote" }),
              h(
                "div",
                { class: "row", style: "gap:7px;flex-wrap:wrap" },
                ...mochiColors.map((c) =>
                  h("button", {
                    class: `mascot-swatch ${color === c ? "on" : ""}`,
                    style: `background:${c}`,
                    title: c,
                    onclick: () => void setColor(c),
                  }),
                ),
                h("label", { class: `mascot-swatch custom ${mochiColors.includes(color) ? "" : "on"}`, title: "Qualquer cor" }, picker),
              ),
              h("div", { class: "row", style: "gap:8px;margin-top:8px" }, h("span", { class: "mascot-sub", text: "Hex" }), hexIn),
            ),
            h(
              "div",
              {},
              h("div", { class: "mascot-label", text: "Acabamento" }),
              h(
                "div",
                { class: "ag-seg" },
                ...FINISH_OPTIONS.map((f) =>
                  h("button", {
                    class: finish === f.id ? "on" : "",
                    text: f.label,
                    onclick: async () => {
                      data.mochiFinish = f.id;
                      mochiFinish = f.id;
                      await saveData();
                      render();
                    },
                  }),
                ),
              ),
            ),
            h(
              "div",
              {},
              h("div", { style: "font-size:11px;color:var(--muted);margin-bottom:5px", text: "Seu Nome" }),
              h(
                "div",
                { class: "row" },
                nameIn,
                h("button", {
                  class: "btn btn-sm",
                  text: "Salvar",
                  onclick: async () => {
                    data.userName = nameIn.value.trim();
                    await saveData();
                    // Empty: the store puts the Windows user name back, as on first start.
                    if (!data.userName) data.userName = (await Bridge.jarvisDataLoad())?.userName ?? "";
                    render();
                  },
                }),
              ),
            ),
            h(
              "div",
              {},
              h("div", { style: "font-size:11px;color:var(--muted);margin-bottom:5px", text: "Nome do Assistente" }),
              h(
                "div",
                { class: "row" },
                assistIn,
                h("button", {
                  class: "btn btn-sm",
                  text: "Salvar",
                  onclick: async () => {
                    data.assistantName = assistIn.value.trim() || "Jarvis";
                    await saveData();
                    render();
                  },
                }),
              ),
            ),
          ),
        ),
      );
      return;
    }

    const mode = State.settings.jarvisBrainMode ?? "auto";
    const groqOk = brainStatus?.groqConfigured ?? false;
    const gemOk = brainStatus?.geminiConfigured ?? false;
    const openaiOk = brainStatus?.openaiConfigured ?? false;
    const claudeOk = brainStatus?.claudeConfigured ?? false;
    const claudeCliOk = brainStatus?.claudeCliLoggedIn ?? false;
    const claudeOauthOk = brainStatus?.claudeOauthConfigured ?? false;
    const gemCliOk = brainStatus?.geminiCliLoggedIn ?? false;
    const gemOauthOk = brainStatus?.geminiOauthConfigured ?? false;

    const brainCard = (id: string, title: string, badge: string, desc: string) =>
      h(
        "div",
        {
          class: `brain-card ${mode === id ? "selected" : ""}`,
          onclick: async () => {
            await saveSettings({ ...State.settings, jarvisBrainMode: id });
            render();
          },
        },
        h(
          "div",
          { class: "row between", style: "margin-bottom:4px" },
          h("span", { style: "font-weight:600;font-size:12.5px", text: title }),
          badge
            ? h("span", {
                class: "brain-tag",
                text: badge,
              })
            : "",
        ),
        h("div", { style: "font-size:11px;color:var(--muted);line-height:1.35", text: desc }),
      );

    const groqIn = h("input", { class: "input", type: "password", style: "width:155px", placeholder: groqOk ? "Nova chave gsk_..." : "gsk_..." }) as HTMLInputElement;
    const gemIn = h("input", { class: "input", type: "password", style: "width:155px", placeholder: gemOk ? "Nova chave AIza..." : "AIza..." }) as HTMLInputElement;
    const openaiIn = h("input", { class: "input", type: "password", style: "width:155px", placeholder: openaiOk ? "Nova chave sk-..." : "sk-..." }) as HTMLInputElement;
    const claudeIn = h("input", { class: "input", type: "password", style: "width:155px", placeholder: claudeOk ? "Nova chave sk-ant-..." : "sk-ant-..." }) as HTMLInputElement;
    const testOut = h("span", { style: "font-size:11.5px;color:var(--muted)" });
    const agents = getAgentsList();

    const PROVIDERS: { id: AiProviderId; avatar: string; name: string; desc: string }[] = [
      { id: "claude", avatar: "C", name: "Claude (Anthropic)", desc: "Conta Claude / Anthropic" },
      { id: "gemini", avatar: "G", name: "Gemini (Google)", desc: "Conta Google / Gemini" },
      { id: "openai", avatar: "O", name: "OpenAI (ChatGPT)", desc: "Conta ChatGPT / OpenAI" },
      { id: "groq", avatar: "Q", name: "Groq", desc: "Conta Groq (Llama & Whisper)" },
    ];

    const renderProviderCard = (p: { id: AiProviderId; avatar: string; name: string; desc: string }) => {
      const list = accounts.filter((a) => a.provider === p.id);
      const activeAcc = list.find((a) => a.active) ?? list[0];
      const isLoggingIn = loggingInProvider === p.id;
      const pillText = list.length > 0 ? `${list.length} conta${list.length > 1 ? "s" : ""}` : "Desconectado";
      const subText = activeAcc
        ? `Ativa: ${activeAcc.label}${activeAcc.email ? ` (${activeAcc.email})` : ""}`
        : p.desc;

      return h(
        "div",
        { class: "provider-card" },
        h(
          "div",
          { class: "provider-card-head" },
          h(
            "div",
            { class: "account-info" },
            h("div", { class: "account-avatar", text: p.avatar }),
            h(
              "div",
              { class: "account-meta" },
              h(
                "div",
                { class: "account-name" },
                h("span", { text: p.name }),
                h("span", { class: `auth-pill ${list.length > 0 ? "connected" : ""}`, text: pillText }),
              ),
              h("div", { class: "account-sub", text: subText }),
            ),
          ),
          h(
            "div",
            { class: "provider-actions" },
            h("button", {
              class: "btn-connect-account",
              disabled: loggingInProvider !== null,
              text: isLoggingIn ? "Aguardando navegador…" : "Entrar / Conectar conta",
              onclick: async () => {
                if (loggingInProvider !== null) return;
                loggingInProvider = p.id;
                authNotice = `Abrindo navegador para conectar ${p.name}…`;
                render();
                try {
                  const created = await Bridge.jarvisLoginAccount(p.id);
                  accounts = (await Bridge.jarvisListAccounts()) ?? [...accounts, created];
                  brainStatus = await Bridge.jarvisBrainStatus();
                  authNotice = `Conta "${created.label}" conectada em ${p.name}.`;
                } catch (e) {
                  authNotice = String(e).replace(/^Error:\s*/, "");
                } finally {
                  loggingInProvider = null;
                  render();
                }
              },
            }),
          ),
        ),
        list.length > 0
          ? h(
              "div",
              { class: "account-list" },
              ...list.map((acc) =>
                h(
                  "div",
                  { class: `account-row ${acc.active ? "active" : ""}` },
                  h(
                    "div",
                    { class: "account-meta" },
                    h(
                      "div",
                      { class: "account-name" },
                      h("span", { text: acc.label }),
                      acc.active ? h("span", { class: "account-badge-active", text: "Ativa" }) : "",
                    ),
                    h("div", {
                      class: "account-sub",
                      text: `${acc.email ? `${acc.email} · ` : ""}${acc.authMethod}`,
                    }),
                  ),
                  h(
                    "div",
                    { class: "account-actions" },
                    !acc.active
                      ? h("button", {
                          class: "btn-switch-account",
                          text: "Usar",
                          onclick: async () => {
                            try {
                              accounts = await Bridge.jarvisSwitchAccount(acc.id);
                              brainStatus = await Bridge.jarvisBrainStatus();
                              authNotice = `Conta "${acc.label}" ativada.`;
                            } catch (e) {
                              authNotice = String(e).replace(/^Error:\s*/, "");
                            }
                            render();
                          },
                        })
                      : "",
                    h("button", {
                      class: "btn-remove-account",
                      text: "Remover",
                      onclick: async () => {
                        try {
                          accounts = await Bridge.jarvisRemoveAccount(acc.id);
                          brainStatus = await Bridge.jarvisBrainStatus();
                          authNotice = `Conta "${acc.label}" removida.`;
                        } catch (e) {
                          authNotice = String(e).replace(/^Error:\s*/, "");
                        }
                        render();
                      },
                    }),
                  ),
                ),
              ),
            )
          : "",
      );
    };

    const hasClaude = claudeOk || claudeCliOk || claudeOauthOk || accounts.some((a) => a.provider === "claude");
    const hasGemini = gemOk || gemCliOk || gemOauthOk || accounts.some((a) => a.provider === "gemini");
    const hasOpenai = openaiOk || accounts.some((a) => a.provider === "openai");
    const hasGroq = groqOk || accounts.some((a) => a.provider === "groq");

    main.append(
      h("div", { class: "row between" }, h("div", { class: "page-title", text: "Ajustes" }), subNav), renderUpdateCard(),
      h(
        "div",
        { class: "brain-banner" },
        h("div", { class: "status-dot green" }),
        h(
          "div",
          {},
          h("div", { style: "font-weight:600;font-size:12.5px", text: mode === "auto" ? "Modo automático" : `Modo: ${mode}` }),
          h("div", { style: "font-size:11px;color:var(--muted)", text: "Usa a primeira IA conectada e passa para a próxima se ela falhar (Gemini → Groq → OpenAI → Claude)." }),
        ),
      ),
      h(
        "div",
        { class: "grid-3" },
        brainCard("auto", "Automático", "Padrão", "Usa contas e chaves disponíveis com fallback."),
        brainCard("claude", "Claude", hasClaude ? "Conectado" : "", "Usa a conta ou chave Claude ativa."),
        brainCard("gemini", "Gemini", hasGemini ? "Conectado" : "", "Usa a conta ou chave Gemini ativa."),
        brainCard("openai", "ChatGPT / OpenAI", hasOpenai ? "Conectado" : "", "Usa a conta ou chave OpenAI ativa."),
        brainCard("groq", "Groq", hasGroq ? "Conectado" : "", "Llama 3.3 rápido e transcrição Whisper."),
        brainCard("ollama", "Local (Ollama)", "", "Executa localmente sem internet."),
      ),
      h(
        "div",
        { class: "auth-section" },
        h(
          "div",
          { class: "auth-section-header" },
          h("span", { text: "Conexão com IAs" }),
          h(
            "div",
            { class: "row", style: "gap:6px" },
            h(
              "div",
              { class: "auth-mode-switch" },
              h("button", {
                class: `auth-mode-btn ${authSubTab === "accounts" ? "on" : ""}`,
                text: `Contas (${accounts.length})`,
                onclick: () => {
                  authSubTab = "accounts";
                  render();
                },
              }),
              h("button", {
                class: `auth-mode-btn ${authSubTab === "apikeys" ? "on" : ""}`,
                text: "API Keys",
                onclick: () => {
                  authSubTab = "apikeys";
                  render();
                },
              }),
            ),
            h("button", {
              class: "btn btn-ghost btn-sm",
              text: "Atualizar",
              onclick: async () => {
                brainStatus = await Bridge.jarvisBrainStatus();
                accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
                authNotice = "Status das contas atualizado.";
                render();
              },
            }),
          ),
        ),
        ...(authSubTab === "accounts"
          ? PROVIDERS.map(renderProviderCard)
          : [
              h(
                "div",
                { class: "provider-row" },
                h(
                  "div",
                  {},
                  h("div", { style: "font-weight:600;color:#fff", text: groqOk ? "● Groq conectada" : "○ Groq desconectada" }),
                  h("div", { style: "font-size:11px;color:var(--muted)", text: "Respostas rápidas e transcrição Whisper." }),
                ),
                h(
                  "div",
                  { class: "row" },
                  groqIn,
                  h("button", {
                    class: "btn btn-ghost btn-sm",
                    text: groqOk ? "Adicionar" : "Conectar",
                    onclick: async () => {
                      const val = groqIn.value.trim();
                      if (!val) return;
                      await Bridge.secretSet("groq-api-key", val);
                      await Bridge.jarvisAddAccountManual("groq", val, "Groq API Key", "api_key");
                      groqIn.value = "";
                      brainStatus = await Bridge.jarvisBrainStatus();
                      accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
                      render();
                    },
                  }),
                ),
              ),
              h(
                "div",
                { class: "provider-row" },
                h(
                  "div",
                  {},
                  h("div", { style: "font-weight:600;color:#fff", text: gemOk ? "● Gemini conectada" : "○ Gemini desconectada" }),
                  h("div", { style: "font-size:11px;color:var(--muted)", text: "Reserva e análise de capturas de tela." }),
                ),
                h(
                  "div",
                  { class: "row" },
                  gemIn,
                  h("button", {
                    class: "btn btn-ghost btn-sm",
                    text: gemOk ? "Adicionar" : "Conectar",
                    onclick: async () => {
                      const val = gemIn.value.trim();
                      if (!val) return;
                      await Bridge.secretSet("google-api-key", val);
                      await Bridge.jarvisAddAccountManual("gemini", val, "Gemini API Key", "api_key");
                      gemIn.value = "";
                      brainStatus = await Bridge.jarvisBrainStatus();
                      accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
                      render();
                    },
                  }),
                ),
              ),
              h(
                "div",
                { class: "provider-row" },
                h(
                  "div",
                  {},
                  h("div", { style: "font-weight:600;color:#fff", text: openaiOk ? "● OpenAI conectada" : "○ OpenAI desconectada" }),
                  h("div", { style: "font-size:11px;color:var(--muted)", text: "Chave OpenAI / ChatGPT API." }),
                ),
                h(
                  "div",
                  { class: "row" },
                  openaiIn,
                  h("button", {
                    class: "btn btn-ghost btn-sm",
                    text: openaiOk ? "Adicionar" : "Conectar",
                    onclick: async () => {
                      const val = openaiIn.value.trim();
                      if (!val) return;
                      await Bridge.jarvisAddAccountManual("openai", val, "OpenAI API Key", "api_key");
                      openaiIn.value = "";
                      brainStatus = await Bridge.jarvisBrainStatus();
                      accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
                      render();
                    },
                  }),
                ),
              ),
              h(
                "div",
                { class: "provider-row" },
                h(
                  "div",
                  {},
                  h("div", { style: "font-weight:600;color:#fff", text: claudeOk ? "● Claude conectada" : "○ Claude desconectada" }),
                  h("div", { style: "font-size:11px;color:var(--muted)", text: "Chave Anthropic / Claude API." }),
                ),
                h(
                  "div",
                  { class: "row" },
                  claudeIn,
                  h("button", {
                    class: "btn btn-ghost btn-sm",
                    text: claudeOk ? "Adicionar" : "Conectar",
                    onclick: async () => {
                      const val = claudeIn.value.trim();
                      if (!val) return;
                      await Bridge.jarvisAddAccountManual("claude", val, "Claude API Key", "api_key");
                      claudeIn.value = "";
                      brainStatus = await Bridge.jarvisBrainStatus();
                      accounts = brainStatus?.accounts ?? (await Bridge.jarvisListAccounts()) ?? [];
                      render();
                    },
                  }),
                ),
              ),
            ]),
        authNotice ? h("div", { style: "font-size:11px;color:var(--muted);padding:0 2px", text: authNotice }) : "",
      ),
      // Failover: another key, then another AI, when the chosen one fails.
      h(
        "div",
        { class: `nw-setting ${(State.settings.jarvisAiFailover ?? true) ? "on" : ""}` },
        h(
          "button",
          {
            class: "nw-setting-head",
            role: "switch",
            "aria-checked": String(State.settings.jarvisAiFailover ?? true),
            onclick: async () => {
              await saveSettings({ ...State.settings, jarvisAiFailover: !(State.settings.jarvisAiFailover ?? true) });
              render();
            },
          },
          h(
            "div",
            { class: "nw-setting-text" },
            h("div", { class: "nw-setting-title", text: "Trocar de IA automaticamente" }),
            h("div", {
              class: "nw-setting-desc",
              text: "Se a IA ativa cair, ficar sem cota ou a chave for recusada, o Jarvis tenta as outras chaves da mesma IA e depois as outras IAs conectadas. A chave que respondeu vira a ativa; a que falhou descansa 15 min.",
            }),
          ),
          h("span", { class: "nw-switch" }, h("span", { class: "nw-knob" })),
        ),
      ),
      // Start with Windows (on by default; the installer turns it on).
      h(
        "div",
        { class: `nw-setting ${State.settings.autostart ? "on" : ""}` },
        h(
          "button",
          {
            class: "nw-setting-head",
            role: "switch",
            "aria-checked": String(Boolean(State.settings.autostart)),
            onclick: async () => {
              await saveSettings({ ...State.settings, autostart: !State.settings.autostart });
              render();
            },
          },
          h(
            "div",
            { class: "nw-setting-text" },
            h("div", { class: "nw-setting-title", text: "Iniciar com o Windows" }),
            h("div", {
              class: "nw-setting-desc",
              text: "O Jarvis abre sozinho quando você entra no Windows, já com permissão de administrador e sem perguntar de novo.",
            }),
          ),
          h("span", { class: "nw-switch" }, h("span", { class: "nw-knob" })),
        ),
      ),
      // Approve each PowerShell/CMD command before it runs.
      h(
        "div",
        { class: `nw-setting ${State.settings.jarvisConfirmSystemActions ? "on" : ""}` },
        h(
          "button",
          {
            class: "nw-setting-head",
            role: "switch",
            "aria-checked": String(Boolean(State.settings.jarvisConfirmSystemActions)),
            onclick: async () => {
              await saveSettings({ ...State.settings, jarvisConfirmSystemActions: !State.settings.jarvisConfirmSystemActions });
              render();
            },
          },
          h(
            "div",
            { class: "nw-setting-text" },
            h("div", { class: "nw-setting-title", text: "Confirmar comandos do sistema" }),
            h("div", { class: "nw-setting-desc", text: "Antes de rodar um comando de PowerShell/CMD, o Jarvis mostra o comando e espera você tocar em Permitir." }),
          ),
          h("span", { class: "nw-switch" }, h("span", { class: "nw-knob" })),
        ),
      ),
      h(
        "div",
        { class: "card row between" },
        h("div", { style: "font-size:11.5px;font-weight:600;color:#fff", text: "Agente padrão" }),
        h(
          "div",
          { class: "agent-chip-group" },
          ...agents.map((ag) =>
            h("button", {
              class: `agent-chip ${selectedAgent === ag.id ? "on" : ""}`,
              title: ag.description,
              text: ag.label,
              onclick: () => {
                setSelectedAgent(ag.id);
                render();
              },
            }),
          ),
        ),
      ),
      h(
        "div",
        { class: "card row between" },
        h(
          "div",
          { class: "row" },
          h("button", {
            class: "btn btn-sm",
            text: "Testar conexão",
            onclick: async () => {
              testOut.textContent = "Consultando…";
              try {
                const r = await Bridge.jarvisAsk("Me conta uma curiosidade rápida em 1 frase.", [], undefined, undefined, selectedAgent);
                testOut.textContent = `[${r.providerUsed}] ${r.text}`;
              } catch (e) {
                testOut.textContent = String(e);
              }
            },
          }),
          testOut,
        ),
        h(
          "div",
          { class: "row" },
          h("button", {
            class: "btn btn-ghost btn-sm",
            text: `Wake word: ${(State.settings.jarvisWakeWordEnabled ?? true) ? "ON" : "OFF"}`,
            onclick: async () => {
              await saveSettings({ ...State.settings, jarvisWakeWordEnabled: !(State.settings.jarvisWakeWordEnabled ?? true) });
              Voice.syncWakeWord();
              render();
            },
          }),
          h("button", {
            class: "btn btn-ghost btn-sm",
            text: `Voz: ${(State.settings.jarvisTtsEnabled ?? true) ? "ON" : "OFF"}`,
            onclick: async () => {
              await saveSettings({ ...State.settings, jarvisTtsEnabled: !(State.settings.jarvisTtsEnabled ?? true) });
              render();
            },
          }),
        ),
      ),
      sectionHead("Voz do Jarvis"),
      renderVoicePicker(),
      sectionHead("Reconhecimento da sua voz"),
      renderWakeEnroll(),
      sectionHead("Microfone"),
      renderMicSettings(),
      sectionHead("Pastas e integrações"),
      renderEditorPicker(),
      renderObsSettings(),
      renderBrowserLogins(),
      renderFolderSettings(),
      sectionHead("Sobre"),
      renderBackup(),
      renderAboutUpdates(),
    );
  }

  // ── Master Render (Compact Pill / Voice Bar / Expanded Notch) ──────────────

  function render() {
    cleanupGraph?.();
    cleanupGraph = null;
    // A redraw of the same page keeps its scroll (it used to jump to the top).
    const view = expanded ? `${activeTab}/${activeTab === "settings" ? settingsSubTab : ""}` : "";
    const keepScroll = view && view === lastView ? (root.querySelector<HTMLElement>(".main")?.scrollTop ?? 0) : 0;
    lastView = view;
    if (keepScroll) {
      queueMicrotask(() => {
        const main = root.querySelector<HTMLElement>(".main");
        if (main) main.scrollTop = keepScroll;
      });
    }
    clear(root);

    const aName = data.assistantName || "Jarvis";
    const mColor = data.mochiColor || "#f4f4f5";
    const outfit = currentOutfit();
    const bState = currentBotState();

    if (!expanded) {
      musicPanel.hide();
      agendaPanel.hide();
      const isListening = Voice.state === "recording" || Voice.state === "transcribing" || Voice.state === "listening";
      const runningAction = hasRunningAction();
      const isExecuting = !isListening && (liveStatus.phase === "executing" || Boolean(runningAction));
      const isThinking = !isListening && !isExecuting && (liveStatus.phase === "thinking" || sending);
      const busy = isListening || isExecuting || isThinking || Boolean(toastReply) || Boolean(waCall);

      // Hidden mode, nothing going on: only the invisible wake strip remains.
      if (notchHidden() && !revealed && !busy) {
        const strip = h("div", { class: "wake-strip", title: "Abrir o Jarvis" });
        const reveal = () => {
          if (expanded || revealed) return;
          revealed = true;
          animatePill = true;
          setWindowCollapsed(false);
          render();
        };
        // mouseenter can be missed when the window resized under a still
        // cursor: any movement over the lip counts too, and a click opens the
        // panel straight away.
        strip.addEventListener("mouseenter", reveal);
        strip.addEventListener("mousemove", reveal);
        strip.addEventListener("mousedown", (e) => {
          e.preventDefault();
          revealed = true;
          setWindowCollapsed(false);
          setExpanded(true);
        });
        root.append(strip);
        pruneDressedMochis();
        pushRect(strip);
        setWindowCollapsed(true);
        return;
      }
      // Something to show: the window must be full size.
      setWindowCollapsed(false);
      if (notchHidden() && busy) animatePill = animatePill || !revealed;

      // A WhatsApp call: the larger bar, with who, how long and the controls
      // (listening / thinking take the bar back while they last).
      if (waCall && !isListening && !isExecuting && !isThinking) {
        const c = waCall;
        const act = (action: string) => (e: Event) => {
          e.stopPropagation();
          void Bridge.whatsappCallAction(action)
            .then((next) => {
              waCall = next.state === "none" ? null : next;
              render();
            })
            .catch(() => {});
        };
        const round = (text: string, action: string, cls: string, title: string) =>
          h("button", { class: `call-pill-btn ${cls}`, text, title, onclick: act(action) });
        const callBar = h(
          "div",
          { class: `notch-voice-bar call-pill ${c.state}` },
          h(
            "div",
            { class: "notch-pill-left", onclick: () => setExpanded(true, "whatsapp"), style: "cursor:pointer;flex:1" },
            h("div", { class: "call-pill-icon", text: "✆" }),
            h(
              "div",
              { class: "notch-status-copy" },
              h("span", { class: "notch-pill-title", text: c.name || "Chamada do WhatsApp" }),
              h("span", { class: "notch-pill-sub", text: c.state === "incoming" ? "Ligando para você…" : c.time || c.status || "Chamando…" }),
            ),
          ),
          h(
            "div",
            { class: "notch-pill-right call-pill-actions" },
            ...(c.state === "incoming"
              ? [round("✕", "decline", "decline", "Recusar"), round("✆", "accept", "accept", "Atender")]
              : [round(c.micOff ? "🔇" : "🎙", "mute", c.micOff ? "on" : "", c.micOff ? "Ativar microfone" : "Silenciar"), round("✆", "hangup", "hangup", "Desligar")]),
          ),
        );
        if (animatePill) {
          callBar.classList.add("appear");
          animatePill = false;
        }
        root.append(callBar);
        pruneDressedMochis();
        pushRect(callBar);
        return;
      }

      if (busy) {
        const botSlot = createDressedMochi(24, mColor, outfit, bState);
        const activeAgentName = getAgentsList().find((a) => a.id === selectedAgent)?.label ?? selectedAgent;
        const barText = isListening
          ? "Ouvindo…"
          : isExecuting
            ? (liveStatus.phase === "executing" && liveStatus.title ? liveStatus.title : "Executando ação…")
            : isThinking
              ? (liveStatus.phase === "thinking" && liveStatus.title ? liveStatus.title : `${aName} está pensando…`)
              : `${aName}: ${toastReply}`;
        const barSub = isExecuting
          ? (
              liveStatus.phase === "executing"
                ? liveStatus.detail
                : runningAction?.description || runningAction?.command || runningAction?.kind || ""
            )
          : isThinking
            ? (liveStatus.detail || `Agente: ${activeAgentName}`)
            : "";
        const barClass = isListening
          ? "notch-voice-bar"
          : isExecuting
            ? "notch-voice-bar executing"
            : isThinking
              ? "notch-voice-bar thinking"
              : "notch-voice-bar";
        const dotClass = isListening
          ? "status-dot"
          : isExecuting
            ? "status-dot green"
            : isThinking
              ? "status-dot thinking"
              : "status-dot green";

        const voiceBar = h(
          "div",
          { class: barClass },
          h(
            "div",
            { class: "notch-pill-left", onclick: () => setExpanded(true, "chat"), style: "cursor:pointer;flex:1" },
            h("div", { class: "notch-mochi-slot" }, botSlot),
            h("div", { class: dotClass }),
            h(
              "div",
              { class: "notch-status-copy" },
              h("span", { class: "notch-pill-title", text: barText }),
              barSub ? h("span", { class: "notch-pill-sub", text: barSub }) : "",
            ),
          ),
          h(
            "div",
            { class: "notch-pill-right" },
            isListening
              ? h("span", { class: "wave" }, h("i"), h("i"), h("i"), h("i"), h("i"))
              : isExecuting
                ? h("span", { class: "exec-spinner", title: "Executando" })
                : isThinking
                  ? h("span", { class: "thinking-dots", title: "Pensando" }, h("i"), h("i"), h("i"))
                  : "",
            h(
              "button",
              {
                class: "icon-action",
                title: "Fechar",
                onclick: (e: Event) => {
                  e.stopPropagation();
                  if (isListening) Voice.stopRecording();
                  toastReply = "";
                  render();
                },
              },
              svgIcon(ICONS.close, 12),
            ),
            h(
              "button",
              {
                class: "icon-action",
                title: "Expandir",
                onclick: (e: Event) => {
                  e.stopPropagation();
                  setExpanded(true, "chat");
                },
              },
              svgIcon(ICONS.external, 12),
            ),
          ),
        );
        if (animatePill) {
          voiceBar.classList.add("appear");
          animatePill = false;
        }
        root.append(voiceBar);
        pruneDressedMochis();
        pushRect(voiceBar);
        if (isThinking || isExecuting) {
          window.setTimeout(() => {
            if (voiceBar.isConnected) pushRect(voiceBar);
          }, 240);
        }
        return;
      }

      const now = new Date();
      const clock = `${now.getHours().toString().padStart(2, "0")}:${now.getMinutes().toString().padStart(2, "0")}`;
      const botSlot = createDressedMochi(20, mColor, outfit, bState);
      // Only while music is actually playing: paused or stopped shows nothing.
      const other = mediaNow && mediaNow.playing && mediaNow.title && mediaNow.app !== "Spotify" ? mediaNow : null;
      const trackLabel = nowPlaying.playing && nowPlaying.title
        ? `${nowPlaying.title}${nowPlaying.artist ? ` · ${nowPlaying.artist}` : ""}`
        : other
          ? `${other.title}${other.artist ? ` · ${other.artist}` : ""} — ${other.app}`
          : "";

      if (State.settings !== widgetSettings) {
        widgetSettings = State.settings;
        widgets.configure(State.settings);
      }
      const pill = h(
        "div",
        {
          class: `notch-pill ${gameModeOn() ? "game" : ""}`,
          style: "max-width:620px",
          onclick: () => setExpanded(true),
        },
        h(
          "div",
          { class: "notch-pill-left" },
          h("div", { class: "notch-mochi-slot" }, botSlot),
          h("div", { class: "status-dot green" }),
          trackLabel
            ? h("span", {
                class: "notch-pill-title",
                style: "max-width:200px;font-size:11px;color:#d1d5db",
                text: trackLabel,
              })
            : "",
        ),
        h("div", { class: "notch-pill-right" }, widgets.el, h("span", { class: "notch-clock", text: clock })),
      );
      if (animatePill) {
        pill.classList.add("appear");
        animatePill = false;
      }
      root.append(pill);
      pruneDressedMochis();
      pushRect(pill);
      return;
    }

    // State 3: Expanded Notch Panel
    const pendingTasks = data.tasks.filter((t) => !t.done).length + data.reminders.length;
    const projCount = data.memories.filter((m) => m.category === "project").length;

    const primaryNav: { id: TabId; label: string; icon: string; badge?: string; activeCheck?: boolean }[] = [
      { id: "home", label: "Início", icon: ICONS.home },
      { id: "chat", label: "Conversa", icon: ICONS.chat },
      { id: "memory", label: "Projetos", icon: ICONS.projects, badge: projCount > 0 ? String(projCount) : undefined },
      { id: "agenda", label: "Agenda", icon: ICONS.calendar, badge: todayEvents.length ? String(todayEvents.length) : undefined },
      { id: "music", label: "Spotify", icon: ICONS.music, badge: nowPlaying.playing ? "●" : undefined },
      { id: "mail", label: "E-mail", icon: ICONS.mail },
      { id: "whatsapp", label: "WhatsApp", icon: ICONS.whatsapp },
      { id: "files", label: "Arquivos", icon: ICONS.fileSearch },
      { id: "notes", label: "Notas", icon: ICONS.notes },
      {
        id: "tasks",
        label: "Organizar",
        icon: ICONS.organize,
        badge: pendingTasks > 0 ? String(pendingTasks) : undefined,
        activeCheck: ORGANIZE_TABS.has(activeTab),
      },
    ];

    const footerNav: { id: TabId; label: string; icon: string; badge?: string }[] = [
      { id: "pc", label: "Meu PC", icon: ICONS.pc },
      // A new version waiting: a dot on Ajustes, where the update card is.
      { id: "settings", label: "Ajustes", icon: ICONS.settings, badge: updateInfo?.version ? "Novo" : undefined },
    ];

    const makeNavButton = (item: { id: TabId; label: string; icon: string; badge?: string; activeCheck?: boolean }) =>
      h(
        "button",
        {
          class: `nav-item ${(item.activeCheck ?? activeTab === item.id) ? "active" : ""}`,
          "data-tab": item.id,
          onclick: () => openTab(item.id),
        },
        h("span", { class: "nav-left" }, svgIcon(item.icon, 14), h("span", { text: item.label })),
        item.badge ? h("span", { class: "nav-badge", text: item.badge }) : "",
      );

    const navList = h(
      "div",
      { class: "nav-list" },
      ...primaryNav.map(makeNavButton),
    );

    const navFooter = h(
      "div",
      { class: "nav-footer" },
      ...footerNav.map(makeNavButton),
    );

    const sidebarBot = createDressedMochi(30, mColor, outfit, bState);

    const health = aiHealth();
    // Activity wins over health; otherwise the AI's real state.
    const busyLabel = Voice.state === "recording" || Voice.state === "listening" ? "Ouvindo…" : sending ? "Pensando…" : "";
    const statusLabel = busyLabel || health.label;
    const statusDot = busyLabel ? "green" : health.state === "ready" ? "green" : health.state === "offline" ? "red" : "idle";
    const statusEl = h(
      "button",
      {
        class: `brand-status ${health.state}`,
        title: busyLabel ? "" : health.detail,
        onclick: () => {
          // Not ready: take the user to where it is fixed.
          if (health.state !== "ready") {
            settingsSubTab = "brain";
            openTab("settings");
          }
        },
      },
      h("span", { class: `status-dot ${statusDot}` }),
      h("span", { text: statusLabel }),
    );

    const sidebar = h(
      "aside",
      { class: "sidebar" },
      h(
        "div",
        { class: "brand" },
        h("div", { class: "brand-mochi" }, sidebarBot),
        h(
          "div",
          { class: "brand-info right" },
          h("span", { class: "brand-name", text: aName }),
          statusEl,
        ),
        h(
          "button",
          {
            class: "icon-action brand-collapse",
            title: "Recolher (Esc)",
            onclick: () => setExpanded(false),
          },
          svgIcon(ICONS.chevronUp, 13),
        ),
      ),
      navList,
      navFooter,
    );

    const main = h("main", { class: "main" });
    if (activeTab !== "music") musicPanel.hide();
    if (activeTab !== "agenda") agendaPanel.hide();
    if (ORGANIZE_TABS.has(activeTab)) {
      renderOrganize(main);
    } else {
      switch (activeTab) {
        case "home": renderHome(main); break;
        case "chat": renderChat(main); break;
        case "memory": renderMemory(main); break;
        case "music": renderMusic(main); break;
        case "agenda":
          main.classList.add("main-flush");
          main.append(agendaPanel.el);
          void agendaPanel.show();
          break;
        case "pc":
          renderPC(main);
          toolTabs.renderPcExtras(main);
          break;
        case "mail": toolTabs.renderMail(main); break;
        case "whatsapp": toolTabs.renderWhatsApp(main); break;
        case "files": toolTabs.renderFiles(main); break;
        case "notes": toolTabs.renderNotes(main); break;
        case "settings": renderSettings(main); break;
      }
    }

    const panel = h("div", { class: `notch-panel ${animateOpen ? "opening" : ""}` }, sidebar, main);
    animateOpen = false;
    root.append(panel);
    pruneDressedMochis();
    pushRect(panel);
  }

  // ── Lifecycle, 60fps Mochi Loop, Voice & Events ────────────────────────────

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && expanded) {
      setExpanded(false);
    }
  });

  await onEvent<string>("tray", (what) => {
    if (what === "settings") setExpanded(true, "settings");
    else if (what === "open") setExpanded(true, "home");
  });

  await onEvent<TabId>("jarvis-open-tab", (tab) => {
    setExpanded(true, tab);
  });

  await onEvent<{ phase?: string; title?: string; detail?: string }>("jarvis-status", (st) => {
    const phase = st?.phase === "executing" ? "executing" : st?.phase === "thinking" ? "thinking" : "idle";
    liveStatus = {
      phase,
      title: st?.title || "",
      detail: st?.detail || "",
    };
    render();
  });

  await onEvent<{ minutes: number; label: string }>("jarvis-start-timer", ({ minutes, label }) => {
    timers.push({
      id: `tm_${Date.now()}`,
      label: label || `${minutes} min`,
      remainingSec: Math.max(1, minutes) * 60,
      running: true,
    });
    render();
  });

  // "Follow the mouse": never pull an open panel to another display.
  await onEvent<null>("screen-changed", () => {
    if (expanded && State.settings.screen === "cursor") return;
    void Bridge.reposition();
  });

  // Hidden mode: the revealed pill goes away once the mouse leaves it.
  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => {
    if (expanded || !notchHidden() || !revealed) return;
    const pill = root.querySelector<HTMLElement>(".notch-pill, .notch-voice-bar");
    if (!pill) return;
    const r = pill.getBoundingClientRect();
    const near = x >= r.left - 28 && x <= r.right + 28 && y <= r.bottom + 28;
    if (near) {
      if (hideTimer != null) {
        window.clearTimeout(hideTimer);
        hideTimer = null;
      }
      return;
    }
    if (hideTimer == null) {
      hideTimer = window.setTimeout(() => {
        hideTimer = null;
        if (expanded || !revealed) return;
        const el = root.querySelector<HTMLElement>(".notch-pill, .notch-voice-bar");
        el?.classList.add("leaving");
        window.setTimeout(() => {
          revealed = false;
          render();
        }, 160);
      }, 550);
    }
  });

  Voice.onCommand = (text) => {
    // "o que tem na minha tela", "olha minha tela", "lê isso na tela"…
    const screen = /\b(minha|na|da|nessa|esta|essa) tela\b|\bprint\b/i.test(text);
    if (sending) {
      // Still answering the previous one: run this one right after, not never.
      queuedVoice = { text, screen };
      void Bridge.jarvisLog(`voice: na fila (Jarvis ocupado): "${text.slice(0, 80)}"`);
      return;
    }
    void askJarvis(text, screen, !expanded, true);
  };

  // Media of any app (YouTube, VLC…): the pill shows it when Spotify is quiet.
  await onEvent<MediaNow | null>("media-now", (m) => {
    mediaNow = m;
    if (!expanded) render();
  });

  // Fixed Jarvis hotkeys: Ctrl+Alt+J (screen) and Ctrl+Alt+K (translate).
  await onEvent<string>("jarvis-hotkey", async (action) => {
    if (action === "jarvisVoice") {
      if (Voice.localWake) void Bridge.wakewordListenNow().catch(() => Voice.toggleRecording());
      else void Voice.toggleRecording();
    } else if (action === "jarvisScreen") {
      void askJarvis("Olhe minha tela e diga, em poucas frases, o que tem de importante nela. Se houver um erro, explique como resolver.", true, !expanded);
    } else if (action === "jarvisTranslate") {
      const text = await Bridge.selectedText();
      if (!text) {
        showToast("Selecione um texto em qualquer app e aperte Ctrl+Alt+K.");
        return;
      }
      const out = await askJarvis(
        `Traduza ou corrija o texto abaixo. Se estiver em português correto, traduza para inglês; se estiver em outro idioma, traduza para português; se for português com erros, corrija. Responda SÓ com o resultado, sem comentários.\n\n${text}`,
        false,
        !expanded,
      );
      if (out) {
        void Bridge.copyText(out);
        showToast(`${out}  (copiado)`, 12000);
      }
    }
  });

  // Reminders: Google Agenda notifications and Organizar → Lembretes.
  await onEvent<{ title: string; body: string }>("jarvis-alert", (a) => {
    Sound.play("question");
    showToast(`🔔 ${a.title} — ${a.body}`, 25000);
    void Voice.speak(`Lembrete: ${a.title}, ${a.body}.`);
  });

  // Routines started by their trigger, and their "say:" steps.
  await onEvent<string>("routine-ran", (msg) => showToast(msg));
  await onEvent<string>("jarvis-say", (msg) => {
    showToast(msg);
    void Voice.speak(msg);
  });

  // A watched WhatsApp contact wrote: said out loud, then the answer is heard
  // ("sim" reads it, "não" leaves it) — no "Jarvis" needed.
  await onEvent<{ contact: string; text: string }>("whatsapp-incoming", (m) => {
    Sound.play("question");
    showToast(`💬 ${m.text}`, 20000);
    chatLog.push({ role: "assistant", content: m.text, meta: "WhatsApp" });
    void Voice.speak(m.text).then(() => {
      if (State.settings.jarvisWakeWordEnabled ?? true) listenForAnswer();
    });
  });

  // Game mode: a game shows FPS for 3 s → quiet notch; when it ends, a summary.
  void Bridge.onPcMetrics((m: PcMetrics) => {
    if (m.fps != null && m.fpsApp) {
      if (!game || game.app !== m.fpsApp) game = { app: m.fpsApp, start: Date.now(), fps: [], misses: 0 };
      game.fps.push(m.fps);
      game.misses = 0;
      if (game.fps.length === 3 && State.settings.jarvisGameMode) {
        soundBeforeGame = State.settings.soundEnabled;
        Sound.setEnabled(false);
        if (!expanded) render();
      }
    } else if (game && ++game.misses >= 6) {
      const g = game;
      game = null;
      if (soundBeforeGame != null) {
        Sound.setEnabled(soundBeforeGame);
        soundBeforeGame = null;
      }
      if (State.settings.jarvisGameMode && g.fps.length >= 30) {
        const mins = Math.round((Date.now() - g.start) / 60000);
        const sorted = [...g.fps].sort((a, b) => a - b);
        const avg = Math.round(g.fps.reduce((a, b) => a + b, 0) / g.fps.length);
        const low = sorted[Math.floor(sorted.length * 0.01)] ?? sorted[0];
        const dur = mins >= 60 ? `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, "0")}` : `${mins} min`;
        showToast(`Sessão de ${g.app}: ${dur} · média ${avg} FPS · 1% low ${low} FPS`, 15000);
      } else if (!expanded) {
        render();
      }
    }
  });

  // "Bom dia": once a day, on the first start.
  if (State.settings.jarvisMorningBriefing) {
    const today = new Date().toDateString();
    let last = "";
    try {
      last = localStorage.getItem("jarvis-briefing-day") ?? "";
    } catch {
      /* private storage: just once per start */
    }
    if (last !== today) {
      try {
        localStorage.setItem("jarvis-briefing-day", today);
      } catch {
        /* ignore */
      }
      window.setTimeout(async () => {
        const text = await Bridge.briefing();
        if (text) {
          showToast(text, 15000);
          void Voice.speak(text);
        }
      }, 4000);
    }
  }
  // Transcription limit and similar: a short note in the notch.
  Voice.onNotice = (text) => showToast(text, 9000);
  // A WhatsApp call, for the collapsed notch's call bar.
  void Bridge.onWhatsappCall((c) => {
    const changed = (waCall?.state ?? "none") !== c.state;
    waCall = c.state === "none" ? null : c;
    if (changed) animatePill = true;
    if (!expanded) render();
  });

  // A new version on GitHub: a note in the notch (once per version) and the card in Ajustes.
  void Bridge.onUpdateAvailable((u) => {
    const first = updateInfo?.version !== u.version;
    updateInfo = u;
    if (first) {
      let seen = "";
      try {
        seen = localStorage.getItem("jarvis-update-seen") ?? "";
      } catch {
        /* ignore */
      }
      if (seen !== u.version) {
        showToast(`Nova versão do Jarvis (${u.version}) disponível — veja em Ajustes.`, 12000);
        try {
          localStorage.setItem("jarvis-update-seen", u.version ?? "");
        } catch {
          /* ignore */
        }
      }
    }
    render();
  });
  void Bridge.onUpdateProgress((p) => {
    updateState =
      p.phase === "installing"
        ? { phase: "installing", pct: 100, note: "Instalando… o Jarvis vai fechar e abrir de novo." }
        : { phase: "downloading", pct: p.total ? Math.round((p.downloaded / p.total) * 100) : 0, note: "" };
    if (expanded && activeTab === "settings") render();
  });

  // The native wake word (the user's enrolled "Jarvis").
  Voice.attachLocal();
  const refreshWake = async () => {
    const st = await Bridge.wakewordStatus();
    if (st) {
      const changed = JSON.stringify(st) !== JSON.stringify(wakeStatus);
      wakeStatus = st;
      // During an enrollment the model is gone (not "active"), but the cloud
      // listener must stay off or it takes the samples for calls.
      if (!enroll) Voice.setLocalWake(st.active);
      // Ajustes shows it: redraw (not during an enrollment, which drives itself).
      if (changed && expanded && activeTab === "settings" && !enroll) render();
    }
  };
  window.setTimeout(() => void refreshWake(), 1500);
  window.setInterval(() => void refreshWake(), 10000);
  refreshWakeSoon = () => {
    void Bridge.wakewordRestart();
    window.setTimeout(() => void refreshWake().then(() => render()), 1200);
  };
  // With media playing, only a closer, louder voice wakes the Jarvis.
  window.setInterval(() => {
    Voice.mediaPlaying = Boolean(mediaNow?.playing) || nowPlaying.playing;
  }, 1500);

  Voice.onStateChange = () => {
    render();
  };
  Voice.syncWakeWord();

  // The mascot at ~30 fps, and not at all while the window is hidden: at 60 fps
  // every frame recomposed the whole (transparent, always-on-top) window.
  let lastFrame = performance.now();
  function animLoop(now: number) {
    if (document.hidden || now - lastFrame < 32) {
      requestAnimationFrame(animLoop);
      return;
    }
    const dt = Math.min(0.05, (now - lastFrame) / 1000);
    lastFrame = now;
    const fin = data.mochiFinish as BodyFinish;
    mochiFinish = BODY_FINISHES.includes(fin) ? fin : "solid";
    tickDressedMochis(dt, data.mochiColor || "#f4f4f5", currentOutfit(), currentBotState());
    requestAnimationFrame(animLoop);
  }
  requestAnimationFrame(animLoop);

  window.setInterval(() => {
    let changed = false;
    for (const tm of timers) {
      if (tm.running && tm.remainingSec > 0) {
        tm.remainingSec--;
        changed = true;
        if (tm.remainingSec === 0) {
          tm.running = false;
          Sound.play("finish");
          void Voice.speak(`Timer ${tm.label} finalizado!`);
        }
      }
    }
    if (!expanded) {
      const clockEl = root.querySelector(".notch-clock");
      if (clockEl) {
        const now = new Date();
        clockEl.textContent = `${now.getHours().toString().padStart(2, "0")}:${now.getMinutes().toString().padStart(2, "0")}`;
      }
    } else if (changed && activeTab === "timers") {
      render();
    }
  }, 1000);

  window.setInterval(async () => {
    void refreshMedia();
    const clip = (await Bridge.jarvisGetClipboard())?.trim() ?? "";
    if (clip && clip !== lastClip) {
      lastClip = clip;
      clipboardHistory.unshift(clip);
      if (clipboardHistory.length > 30) clipboardHistory.pop();
      if (expanded && activeTab === "clipboard") render();
    }
  }, 2500);

  // Today's calendar for Home: now, then every minute.
  void refreshTodayEvents();
  window.setInterval(() => void refreshTodayEvents(), 60_000);

  render();
}
