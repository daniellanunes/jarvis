// Dev harness: runs the real Jarvis notch in a plain browser with a mocked
// Tauri bridge and sample data, so the UI can be designed and screenshotted
// without the app. Not part of the app bundle.
//   /dev/notch-preview.html            → compact
//   /dev/notch-preview.html?open=music → expanded on a tab

const params = new URLSearchParams(location.search);
// Several listeners per event, as in Tauri.
const listeners = new Map<string, ((e: { event: string; payload: unknown }) => void)[]>();

const cover = (seed: string, hue: number) =>
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns='http://www.w3.org/2000/svg' width='300' height='300'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${hue},45%,45%)'/><stop offset='1' stop-color='hsl(${(hue + 40) % 360},50%,18%)'/></linearGradient></defs><rect width='300' height='300' fill='url(#g)'/><text x='24' y='270' font-family='Segoe UI' font-size='26' font-weight='600' fill='rgba(255,255,255,.85)'>${seed}</text></svg>`,
  );

const tracks = [
  { kind: "track", uri: "spotify:track:1", id: "1", name: "Out of Time", subtitle: "The Weeknd · Dawn FM", image: cover("Dawn FM", 200), durationMs: 214000 },
  { kind: "track", uri: "spotify:track:2", id: "2", name: "Sacrifice", subtitle: "The Weeknd · Dawn FM", image: cover("Dawn FM", 200), durationMs: 188000 },
  { kind: "track", uri: "spotify:track:3", id: "3", name: "Oceano", subtitle: "Djavan · Djavan Ao Vivo", image: cover("Djavan", 20), durationMs: 276000 },
  { kind: "track", uri: "spotify:track:4", id: "4", name: "Is There Someone Else?", subtitle: "The Weeknd · Dawn FM", image: cover("Dawn FM", 200), durationMs: 199000 },
];
const playlists = [
  { kind: "playlist", uri: "spotify:playlist:a", id: "a", name: "Foco total", subtitle: "Ana · 48 músicas", image: cover("Foco", 150), durationMs: 0 },
  { kind: "playlist", uri: "spotify:playlist:b", id: "b", name: "Treino", subtitle: "Ana · 62 músicas", image: cover("Treino", 0), durationMs: 0 },
  { kind: "playlist", uri: "spotify:playlist:c", id: "c", name: "MPB de domingo", subtitle: "Ana · 35 músicas", image: cover("MPB", 40), durationMs: 0 },
  { kind: "playlist", uri: "spotify:playlist:d", id: "d", name: "Lo-fi para codar", subtitle: "Spotify · 120 músicas", image: cover("Lo-fi", 260), durationMs: 0 },
  { kind: "playlist", uri: "spotify:playlist:e", id: "e", name: "Descobertas da semana", subtitle: "Spotify · 30 músicas", image: cover("Weekly", 300), durationMs: 0 },
];
let player = {
  playing: true, title: "Out of Time", artist: "The Weeknd", album: "Dawn FM", image: cover("Dawn FM", 200),
  uri: "spotify:track:1", contextUri: "spotify:playlist:a", progressMs: 71000, durationMs: 214000,
  shuffle: false, repeat: "off", volume: 64, device: "MEU-PC",
};

const now = Math.floor(Date.now() / 1000);
const data = {
  userName: "Ana", assistantName: "Jarvis", accentColor: "#84cc16", mochiColor: "#f4f4f5", mochiOutfit: "none",
  memories: [
    { id: "mem_root", label: "Ana", category: "user", createdAt: now },
    { id: "hub_projects", label: "Projetos", category: "pref", createdAt: now },
    { id: "proj_jarvis", label: "jarvis", category: "project", notes: "Caminho: D:\\Projetos\\jarvis\nStack: Tauri / Rust, TypeScript", createdAt: now },
    { id: "proj_aluga", label: "aluga", category: "project", notes: "Caminho: D:\\Projetos\\aluga\nStack: React, TypeScript", createdAt: now },
    { id: "proj_nvcash", label: "nvcash-1.0", category: "project", notes: "Caminho: D:\\Projetos\\nvcash-1.0\nStack: Next.js", createdAt: now },
    { id: "proj_notifica", label: "notifica", category: "project", notes: "Caminho: D:\\Projetos\\notifica\nStack: Python", createdAt: now },
    { id: "tech_ts", label: "TypeScript", category: "tech", createdAt: now },
    { id: "mem_1", label: "Prefere respostas curtas", category: "pref", createdAt: now },
  ],
  edges: [
    { from: "mem_root", to: "hub_projects", label: "projetos" },
    { from: "hub_projects", to: "proj_jarvis", label: "projeto" },
    { from: "hub_projects", to: "proj_aluga", label: "projeto" },
    { from: "proj_jarvis", to: "tech_ts", label: "usa" },
  ],
  tasks: [
    { id: "t1", title: "Revisar PR do notifica", done: false, createdAt: now },
    { id: "t2", title: "Configurar app do Spotify", done: false, createdAt: now },
    { id: "t3", title: "Backup do Obsidian", done: true, createdAt: now },
  ],
  reminders: [{ id: "r1", title: "Reunião com cliente", time: "15:30", done: false }],
  habits: [{ id: "h1", name: "Beber água", streak: 4, doneToday: false }],
  routines: [{ id: "rot_work", name: "Modo Dev", description: "Abre VS Code, Terminal e Spotify", targets: ["vscode", "terminal", "spotify"] }],
};

// Calendar: a few events around today, in local time.
const at = (dayOffset: number, h: number, m = 0) => {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  d.setHours(h, m, 0, 0);
  return d.toISOString();
};
const dayOf = (dayOffset: number) => {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};
let calEvents = [
  { key: "p|1", title: "Daily do time", start: at(0, 9, 30), end: at(0, 9, 45), allDay: false, location: "", description: "", calendar: "Trabalho", color: "#4285f4", link: "https://calendar.google.com", meet: "https://meet.google.com/abc" },
  { key: "p|2", title: "Reunião com cliente — Aluga", start: at(0, 15, 30), end: at(0, 16, 30), allDay: false, location: "Escritório", description: "", calendar: "Trabalho", color: "#4285f4", link: "https://calendar.google.com", meet: "" },
  { key: "p|3", title: "Academia", start: at(0, 19), end: at(0, 20), allDay: false, location: "", description: "", calendar: "Pessoal", color: "#33b679", link: "x", meet: "" },
  { key: "p|4", title: "Aniversário da Ana", start: dayOf(1), end: dayOf(2), allDay: true, location: "", description: "", calendar: "Pessoal", color: "#e67c73", link: "x", meet: "" },
  { key: "p|5", title: "Deploy do notifica", start: at(1, 14), end: at(1, 15), allDay: false, location: "", description: "", calendar: "Trabalho", color: "#4285f4", link: "x", meet: "" },
  { key: "p|6", title: "Dentista", start: at(3, 10), end: at(3, 11), allDay: false, location: "Clínica Sorriso", description: "", calendar: "Pessoal", color: "#33b679", link: "x", meet: "" },
];

const answers: Record<string, (args: any) => unknown> = {
  list_monitors: () => [
    { key: "at:0,0|DELL U2722D|1920x1080", label: "DELL U2722D — 1920×1080 at 0,0" },
    { key: "at:1920,0|LG 24MK|1920x1080", label: "LG 24MK — 1920×1080 at 1920,0" },
  ],
  calendar_status: () => ({ configured: true, connected: params.get("calendar") !== "setup", email: "ana@exemplo.com", project: "206533512741" }),
  calendar_events: ({ from }) => calEvents.filter((e) => (e.allDay ? e.start : dayOf(0)) >= from || true),
  calendar_create: (ev) => {
    const e = { key: `p|${Date.now()}`, title: ev.title, start: ev.allDay ? ev.start : new Date(ev.start).toISOString(), end: ev.allDay ? ev.start : new Date(ev.end).toISOString(), allDay: Boolean(ev.allDay), location: ev.location ?? "", description: "", calendar: "Pessoal", color: "#33b679", link: "x", meet: "" };
    calEvents = [...calEvents, e].sort((a, b) => a.start.localeCompare(b.start));
    return e;
  },
  calendar_delete: ({ key }) => {
    calEvents = calEvents.filter((e) => e.key !== key);
    return null;
  },
  // WhatsApp (fictional chats).
  whatsapp_state: () => ({ state: "ready", qr: null }),
  whatsapp_chats: () => [
    { title: "Ana Souza", last: "Combinado, chego às 8!", time: "14:02", unread: 2 },
    { title: "Família", last: "Mãe: Domingo tem almoço aqui", time: "13:40", unread: 5 },
    { title: "Carlos (trabalho)", last: "Você: Mandei o relatório", time: "11:15", unread: 0 },
    { title: "Mensagens para mim", last: "Você: comprar pão", time: "ontem", unread: 0 },
  ],
  whatsapp_thread: ({ chat }) => ({
    title: chat,
    messages: [
      { when: "13:50", from: chat, text: "Oi! Vamos jantar hoje?", mine: false },
      { when: "13:58", from: "Você", text: "Bora! Que horas?", mine: true },
      { when: "14:01", from: chat, text: "Às 8 no lugar de sempre?", mine: false },
      { when: "14:02", from: chat, text: "Combinado, chego às 8!", mine: false },
    ],
  }),
  whatsapp_send: () => "Mensagem enviada.",
  gmail_list: () => [
    { id: "m1", threadId: "t1", from: "Ana Souza <ana@exemplo.com>", subject: "Proposta do projeto — versão final", date: new Date().toUTCString(), snippet: "Oi! Segue a versão final da proposta com os ajustes que conversamos…", unread: true },
    { id: "m2", threadId: "t2", from: "GitHub <noreply@github.com>", subject: "[jarvis] CI falhou no main", date: new Date(Date.now() - 864e5).toUTCString(), snippet: "Run failed: build (windows-latest)", unread: false },
    { id: "m3", threadId: "t3", from: "Banco <avisos@banco.com>", subject: "Sua fatura chegou", date: new Date(Date.now() - 2 * 864e5).toUTCString(), snippet: "Valor e vencimento da fatura de novembro.", unread: false },
  ],
  gmail_read: ({ id }) => ({ id, threadId: "t1", from: "Ana Souza <ana@exemplo.com>", to: "voce@exemplo.com", subject: "Proposta do projeto — versão final", date: new Date().toUTCString(), snippet: "", unread: false, messageId: "<x@y>", body: "Oi!\n\nSegue a versão final da proposta com os ajustes que conversamos.\n\nAbraços,\nAna" }),
  audio_state: () => ({ outputs: [
    { id: "1", name: "Fones de ouvido (3- Fuxi-H4 )", default: false },
    { id: "2", name: "Alto-falantes (Realtek High Definition Audio)", default: true },
    { id: "3", name: "1 - LG FULL HD (AMD High Definition Audio Device)", default: false },
  ], volume: 85, muted: false, micMuted: false, micName: "Microfone (3- Fuxi-H4 )" }),
  audio_set_output: () => null,
  audio_set_mic_mute: () => null,
  net_speed_test: () => 312.4,
  files_warm: () => null,
  files_search: () => [
    { path: "C:\\Users\\voce\\Documents\\Contratos\\Contrato março 2026.pdf", name: "Contrato março 2026.pdf", dir: "C:\\Users\\voce\\Documents\\Contratos", isDir: false, size: 482133, modified: Date.now() / 1000 - 86400 * 20 },
    { path: "C:\\Users\\voce\\Downloads\\contrato-assinado.pdf", name: "contrato-assinado.pdf", dir: "C:\\Users\\voce\\Downloads", isDir: false, size: 1203411, modified: Date.now() / 1000 - 86400 * 3 },
    { path: "C:\\Users\\voce\\Documents\\Contratos", name: "Contratos", dir: "C:\\Users\\voce\\Documents", isDir: true, size: 0, modified: Date.now() / 1000 - 86400 },
  ],
  files_open: ({ reveal }) => (reveal ? "Mostrando na pasta" : "Abrindo"),
  notes_search: () => [
    { path: "projetos/jarvis.md", title: "jarvis", snippet: "Notch: clima, uso do PC e medidor de FPS", modified: Date.now() / 1000 - 3600 },
    { path: "ferramentas/rea.md", title: "rea", snippet: "REA = Reverse Engineer Anything", modified: Date.now() / 1000 - 86400 * 2 },
  ],
  notes_read: () => "# jarvis\n\n- Notch com clima, PC e FPS\n- Gmail só quando pedir",
  notes_write: ({ title }) => `Jarvis/${title}.md`,
  jarvis_github_overview: () => ({ login: "voce", fetchedAt: Date.now(),
    myPRs: [{ id: "voce/jarvis#12", title: "Notch: widgets de clima e FPS", url: "x", repo: "voce/jarvis", number: 12, isDraft: false, ci: "success", review: "approved" }],
    toReview: [{ id: "time/api#88", title: "Refatora autenticação", url: "x", repo: "time/api", number: 88, isDraft: false, ci: "pending", review: "unknown" }],
    mainCI: [{ repo: "voce/site", url: "x", branch: "main", ci: "failure" }] }),
  secret_present: () => true,
  jarvis_briefing: () => "Bom dia! Agora faz 31 graus, parcialmente nublado, com máxima de 31. Você tem 2 compromissos hoje: o primeiro é Daily às 09:30.",
  media_now: () => null,
  wakeword_status: () => ({ active: params.has("enrolled"), enrolled: params.has("enrolled"), samples: 0, samplesWanted: 6, error: "" }),
  wakeword_restart: () => null,
  weather_now: (a: any) => ({ place: a?.city ? `${a.city}, Pernambuco` : "Recife, Pernambuco", temp: 30.9, feels: 35.5, max: 31.3, min: 25.8, rainChance: 40, code: 2, isDay: true, kind: params.get("wkind") ?? "partly", label: "Parcialmente nublado" }),
  boot: () => ({ settings: { jarvisSpotifyClientId: "x", ...(params.has("widgets") ? { jarvisNotchWeather: true, jarvisWeatherCity: "Recife", jarvisNotchPc: true, jarvisNotchFps: true, jarvisNotchNet: true, jarvisGameMode: params.has("game") } : {}) }, screen: { x: 0, y: 0, width: 1920, height: 1080, scale: 1 }, version: "dev", hookPath: "", cursorPoll: true }),
  jarvis_data_load: () => data,
  jarvis_brain_status: () => ({
    groqConfigured: true, geminiConfigured: true, openaiConfigured: false, claudeConfigured: true, ollamaUrl: "", brainMode: "auto",
    claudeCliInstalled: true, claudeCliLoggedIn: true, geminiCliInstalled: false, geminiCliLoggedIn: false,
    claudeOauthConfigured: true, geminiOauthConfigured: true, availableProviders: ["auto", "gemini", "groq", "claude"],
    availableAgents: [], accounts: [
      { id: "a1", provider: "gemini", label: "Conta pessoal", authMethod: "oauth", email: "ana@exemplo.com", active: true, createdAt: now },
      { id: "a2", provider: "groq", label: "Groq", authMethod: "api_key", active: true, createdAt: now },
    ],
  }),
  jarvis_list_accounts: () => [],
  jarvis_now_playing: () => ({ title: player.title, artist: player.artist, album: player.album, playing: player.playing, coverUrl: player.image }),
  jarvis_pc_stats: () => ({ cpuPercent: 23, memUsedGb: 11.2, memTotalGb: 32, memPercent: 35, diskFreeGb: 412, diskTotalGb: 1000, diskPercent: 59 }),
  jarvis_get_clipboard: () => "",
  spotify_api_status: () => ({ clientIdSet: params.get("spotify") !== "setup", connected: params.get("spotify") !== "setup" && params.get("spotify") !== "login", redirectUri: "http://127.0.0.1:43821/callback" }),
  spotify_player_state: () => player,
  spotify_my_playlists: () => playlists,
  spotify_collection_tracks: () => tracks,
  spotify_search: () => [...tracks.slice(0, 3), playlists[3], playlists[1]],
  spotify_recent: () => [...tracks, ...tracks.map((t) => ({ ...t, uri: t.uri + "r" }))],
  spotify_liked_count: () => 412,
  spotify_play_list: ({ uris, offset }) => answers.spotify_play_uri({ uri: uris[offset] }),
  spotify_devices: () => [{ id: "d1", name: "MEU-PC", kind: "Computer", active: true, volume: 64 }, { id: "d2", name: "iPhone", kind: "Smartphone", active: false, volume: 80 }],
  spotify_play_uri: ({ uri }) => {
    const t = tracks.find((x) => x.uri === uri);
    if (t) player = { ...player, title: t.name, artist: t.subtitle.split(" · ")[0], uri, progressMs: 0, durationMs: t.durationMs, playing: true };
    return null;
  },
  spotify_api_control: ({ action, value }) => {
    if (action === "pause") player = { ...player, playing: false };
    if (action === "play") player = { ...player, playing: true };
    if (action === "shuffle") player = { ...player, shuffle: value > 0 };
    if (action === "volume") player = { ...player, volume: value };
    if (action === "seek") player = { ...player, progressMs: value };
    return null;
  },
  jarvis_ask: async ({ prompt }) => {
    await new Promise((r) => setTimeout(r, 900));
    return { text: `Pronto: ${prompt}.`, providerUsed: "Atalho", actions: [], pendingActions: [] };
  },
};

(window as any).__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "island" }, currentWebview: { windowLabel: "island", label: "island" } },
  transformCallback: (cb: unknown) => cb,
  invoke: async (cmd: string, args: any) => {
    if (cmd === "plugin:event|listen") {
      listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
      return listeners.size;
    }
    if (cmd.startsWith("plugin:")) return null;
    const fn = answers[cmd];
    return fn ? fn(args ?? {}) : null;
  },
};
if (params.has("widgets")) {
  let t = 0;
  setInterval(() => {
    t++;
    (window as any).__emit?.("pc-metrics", {
      cpu: 19 + (t % 5) * 4, ram: 65, ramUsedGb: 10.4, ramTotalGb: 15.9, gpu: 78 + (t % 3) * 6,
      disk: 83, diskFreeGb: 154, diskTotalGb: 931,
      cpuName: "AMD Ryzen 5 5600 6-Core Processor", gpuName: "AMD Radeon RX 6650 XT",
      fps: params.get("widgets") === "fps" ? 141 + (t % 4) : null, fpsApp: "eldenring", fpsAvailable: true,
      pingMs: params.get("net") === "off" ? null : 19 + (t % 3), online: params.get("net") !== "off", netOn: true, cpuTemp: null, gpuTemp: null, sensors: false,
    });
  }, 1000);
}
(window as any).__emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((fn) => fn({ event, payload }));

const { startJarvisNotch } = await import("../src/jarvis/notch_app");
await startJarvisNotch(document.getElementById("root")!);

const open = params.get("open");
if (open) {
  // Expand the notch on the requested tab, as a click on the pill would.
  setTimeout(() => (document.querySelector(".notch-pill") as HTMLElement | null)?.click(), 50);
  setTimeout(() => (document.querySelector(`[data-tab="${open}"]`) as HTMLElement | null)?.click(), 300);
}

// ── Helpers for README screenshots ────────────────────────────────────────────
const click = (sel: string) => (document.querySelector(sel) as HTMLElement | null)?.click();
const sub = params.get("sub");
if (sub) {
  // A settings / organize sub-tab by its button label ("Notch", "Rotinas"…).
  setTimeout(() => {
    const b = [...document.querySelectorAll<HTMLElement>("button")].find((x) => x.textContent?.trim() === sub);
    b?.click();
  }, 700);
}
const wachat = params.get("wachat");
if (wachat) {
  // Opens a WhatsApp chat by its name (the WhatsApp tab must be open).
  setTimeout(() => {
    const row = [...document.querySelectorAll<HTMLElement>(".tt-row")].find((r) => r.textContent?.includes(wachat));
    row?.click();
  }, 1200);
}
const fq = params.get("fq");
if (fq) {
  setTimeout(() => {
    const input = document.querySelector<HTMLInputElement>(".tt-search");
    if (!input) return;
    input.value = fq;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
  }, 900);
}
const scroll = params.get("scroll");
if (scroll) {
  setTimeout(() => {
    const main = document.querySelector<HTMLElement>(".main");
    if (!main) return;
    if (scroll === "end") main.scrollTop = main.scrollHeight;
    else {
      const el = [...main.querySelectorAll<HTMLElement>("*")].find((x) => x.textContent?.trim() === scroll);
      if (el) main.scrollTop = el.offsetTop - 16;
    }
  }, 1400);
}
if (params.has("alert")) {
  setTimeout(() => (window as any).__emit("jarvis-alert", { title: "Reunião com cliente", body: "em 10 minutos · 15:30" }), 1200);
}
if (params.has("mediaapp")) {
  setTimeout(() => (window as any).__emit("media-now", { title: "Lo-fi beats to relax", artist: "", playing: true, app: "YouTube" }), 600);
}
void click;
if (params.has("shot")) {
  // Screenshots: a plain dark page around the notch, in light or dark mode.
  const st = document.createElement("style");
  st.textContent = "html,body{background:#09090b !important}";
  document.head.append(st);
}
