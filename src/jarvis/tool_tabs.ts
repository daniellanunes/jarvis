// The newer notch tabs: E-mail (Gmail, loaded only when opened), Arquivos,
// Notas (Obsidian), plus the Áudio / Internet card of Meu PC.
// Each keeps its own state between renders; `rerender` redraws the panel.

import {
  Bridge,
  type AudioState,
  type FileHit,
  type MailFull,
  type MailItem,
  type NoteHit,
  type WaChat,
  type WaState,
  type WaThread,
  type WaCall,
} from "../core/bridge";
import { lineIcon } from "./line_icons";
import { Sound } from "../core/sound";
import { clear, h } from "../views/dom";

export interface ToolTabsCtx {
  rerender(): void;
  /** True while this tab is the one on screen (async results check it). */
  isOpen(tab: string): boolean;
  sectionHead(title: string, action?: { label: string; onclick: () => void }): HTMLElement;
  /** Sends a request to Jarvis (chat). */
  ask(prompt: string): void;
  /** A short note in the notch. */
  toast?(text: string): void;
}

const fmtDate = (unix: number) =>
  unix ? new Date(unix * 1000).toLocaleDateString("pt-BR", { day: "2-digit", month: "short", year: "numeric" }) : "";

const fmtSize = (n: number) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n >= 1e3 ? `${Math.round(n / 1e3)} KB` : `${n} B`;

/** "Ana Souza <ana@x.com>" → "Ana Souza". */
const senderName = (from: string) => from.replace(/<[^>]*>/, "").replace(/"/g, "").trim() || from;

function mailDate(raw: string): string {
  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
}

function searchBar(placeholder: string, value: string, onSearch: (q: string) => void, autofocus = true): HTMLElement {
  const input = h("input", { class: "input tt-search", placeholder, value }) as HTMLInputElement;
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") onSearch(input.value.trim());
  });
  if (autofocus) queueMicrotask(() => input.focus());
  return h("div", { class: "tt-searchbar" }, input, h("button", { class: "btn", text: "Buscar", onclick: () => onSearch(input.value.trim()) }));
}

const note = (text: string, err = false) => h("div", { class: `tt-note ${err ? "err" : ""}`, text });

export function createToolTabs(ctx: ToolTabsCtx) {
  // ── E-mail ────────────────────────────────────────────────────────────────
  const mail = {
    query: "in:inbox",
    items: null as MailItem[] | null,
    loading: false,
    error: "",
    open: null as MailFull | null,
    loadingId: "",
  };

  async function loadMail(q: string) {
    mail.query = q || "in:inbox";
    mail.loading = true;
    mail.error = "";
    mail.open = null;
    ctx.rerender();
    try {
      mail.items = await Bridge.gmailList(mail.query);
    } catch (e) {
      mail.error = String(e).replace(/^Error:\s*/, "");
      mail.items = null;
    }
    mail.loading = false;
    if (ctx.isOpen("mail")) ctx.rerender();
  }

  function renderMail(main: HTMLElement) {
    // Nothing is fetched until this tab is opened (the user asked for it).
    if (mail.items == null && !mail.loading && !mail.error) void loadMail(mail.query);

    if (mail.open) {
      const m = mail.open;
      main.append(
        h(
          "div",
          { class: "tt-mail-head" },
          h("button", { class: "link-btn", text: "‹ Voltar", onclick: () => ((mail.open = null), ctx.rerender()) }),
          h("div", { class: "tt-mail-subject", text: m.subject || "(sem assunto)" }),
          h("div", { class: "tt-mail-meta", text: `${senderName(m.from)} · ${mailDate(m.date)}` }),
          h(
            "div",
            { class: "tt-actions" },
            h("button", { class: "btn btn-sm", text: "Resumir", onclick: () => ctx.ask(`Resuma este e-mail (id ${m.id}) em poucas linhas.`) }),
            h("button", {
              class: "btn btn-sm",
              text: "Responder com Jarvis",
              onclick: () => ctx.ask(`Escreva um rascunho de resposta para o e-mail id ${m.id} de ${senderName(m.from)}. Me mostre o texto antes; não envie.`),
            }),
          ),
        ),
        h("pre", { class: "tt-mail-body", text: m.body || m.snippet }),
      );
      return;
    }

    main.append(
      ctx.sectionHead("E-mail", { label: "Atualizar", onclick: () => void loadMail(mail.query) }),
      h(
        "div",
        { class: "tt-chips" },
        ...[
          ["Caixa de entrada", "in:inbox"],
          ["Não lidos", "is:unread in:inbox"],
          ["Importantes", "is:important newer_than:7d"],
          ["Com anexo", "has:attachment newer_than:30d"],
        ].map(([label, q]) =>
          h("button", { class: `tt-chip ${mail.query === q ? "on" : ""}`, text: label, onclick: () => void loadMail(q) }),
        ),
      ),
      searchBar("Buscar no Gmail — ex.: from:ana fatura", mail.query.startsWith("in:inbox") || mail.query.includes(":") ? "" : mail.query, (q) => void loadMail(q), false),
    );
    if (mail.loading) return void main.append(note("Carregando e-mails…"));
    if (mail.error) {
      main.append(note(mail.error, true));
      return;
    }
    const items = mail.items ?? [];
    if (!items.length) return void main.append(note("Nenhum e-mail aqui."));
    main.append(
      h(
        "div",
        { class: "tt-list" },
        ...items.map((m) =>
          h(
            "button",
            {
              class: `tt-row tt-mail ${m.unread ? "unread" : ""}`,
              onclick: async () => {
                mail.loadingId = m.id;
                ctx.rerender();
                try {
                  mail.open = await Bridge.gmailRead(m.id);
                } catch (e) {
                  mail.error = String(e);
                }
                mail.loadingId = "";
                if (ctx.isOpen("mail")) ctx.rerender();
              },
            },
            h(
              "div",
              { class: "tt-row-top" },
              h("span", { class: "tt-row-title", text: senderName(m.from) }),
              h("span", { class: "tt-row-meta", text: mail.loadingId === m.id ? "abrindo…" : mailDate(m.date) }),
            ),
            h("div", { class: "tt-row-sub", text: m.subject || "(sem assunto)" }),
            h("div", { class: "tt-row-snippet", text: m.snippet }),
          ),
        ),
      ),
    );
  }

  // ── Arquivos ──────────────────────────────────────────────────────────────
  const files = { query: "", hits: null as FileHit[] | null, loading: false, error: "", flash: "" };

  async function searchFiles(q: string) {
    files.query = q;
    if (!q) return;
    files.loading = true;
    files.error = "";
    ctx.rerender();
    try {
      files.hits = await Bridge.filesSearch(q);
    } catch (e) {
      files.error = String(e).replace(/^Error:\s*/, "");
    }
    files.loading = false;
    if (ctx.isOpen("files")) ctx.rerender();
  }

  function fileIcon(f: FileHit): string {
    if (f.isDir) return "📁";
    const ext = f.name.split(".").pop()?.toLowerCase() ?? "";
    if (["pdf"].includes(ext)) return "📕";
    if (["png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "svg"].includes(ext)) return "🖼";
    if (["mp4", "mkv", "mov", "avi", "webm"].includes(ext)) return "🎞";
    if (["mp3", "wav", "flac", "m4a", "ogg"].includes(ext)) return "🎵";
    if (["xls", "xlsx", "csv", "ods"].includes(ext)) return "📊";
    if (["doc", "docx", "odt", "txt", "md", "rtf"].includes(ext)) return "📄";
    if (["zip", "rar", "7z", "tar", "gz"].includes(ext)) return "🗜";
    if (["exe", "msi"].includes(ext)) return "⚙";
    return "📄";
  }

  function renderFiles(main: HTMLElement) {
    void Bridge.filesWarm();
    main.append(
      ctx.sectionHead("Arquivos"),
      searchBar("Procurar no PC — ex.: pdf contrato março", files.query, (q) => void searchFiles(q)),
      h("div", { class: "tt-hint", text: "Busca pelo nome em Área de Trabalho, Documentos, Downloads, Imagens, Vídeos, Música, OneDrive e na pasta de projetos." }),
    );
    if (files.flash) main.append(note(files.flash));
    if (files.loading) return void main.append(note("Procurando…"));
    if (files.error) return void main.append(note(files.error, true));
    if (files.hits == null) return;
    if (!files.hits.length) return void main.append(note("Nada encontrado com esse nome."));
    main.append(
      h(
        "div",
        { class: "tt-list" },
        ...files.hits.map((f) =>
          h(
            "div",
            { class: "tt-row tt-file" },
            h("span", { class: "tt-file-ico", text: fileIcon(f) }),
            h(
              "div",
              { class: "tt-file-info" },
              h("div", { class: "tt-row-title", text: f.name }),
              h("div", { class: "tt-row-snippet", text: `${f.dir} · ${f.isDir ? "pasta" : fmtSize(f.size)} · ${fmtDate(f.modified)}` }),
            ),
            h(
              "div",
              { class: "tt-actions" },
              h("button", {
                class: "btn btn-sm",
                text: "Abrir",
                onclick: async () => {
                  files.flash = await Bridge.filesOpen(f.path, false).catch((e) => String(e));
                  ctx.rerender();
                },
              }),
              h("button", {
                class: "btn btn-sm btn-ghost",
                text: "Pasta",
                onclick: async () => {
                  files.flash = await Bridge.filesOpen(f.path, true).catch((e) => String(e));
                  ctx.rerender();
                },
              }),
            ),
          ),
        ),
      ),
    );
  }

  // ── Notas ─────────────────────────────────────────────────────────────────
  const notes = { query: "", hits: null as NoteHit[] | null, loading: false, error: "", open: null as { path: string; text: string } | null, flash: "" };

  async function searchNotes(q: string) {
    notes.query = q;
    notes.loading = true;
    notes.error = "";
    notes.open = null;
    ctx.rerender();
    try {
      notes.hits = await Bridge.notesSearch(q);
    } catch (e) {
      notes.error = String(e).replace(/^Error:\s*/, "");
    }
    notes.loading = false;
    if (ctx.isOpen("notes")) ctx.rerender();
  }

  function renderNotes(main: HTMLElement) {
    if (notes.hits == null && !notes.loading && !notes.error) void searchNotes("");
    if (notes.open) {
      const n = notes.open;
      main.append(
        h(
          "div",
          { class: "tt-mail-head" },
          h("button", { class: "link-btn", text: "‹ Voltar", onclick: () => ((notes.open = null), ctx.rerender()) }),
          h("div", { class: "tt-mail-subject", text: n.path.replace(/\.md$/i, "") }),
          h(
            "div",
            { class: "tt-actions" },
            h("button", { class: "btn btn-sm", text: "Abrir no Obsidian", onclick: () => void Bridge.notesOpen(n.path) }),
            h("button", { class: "btn btn-sm", text: "Resumir", onclick: () => ctx.ask(`Leia a nota "${n.path}" do Obsidian e resuma em poucas linhas.`) }),
          ),
        ),
        h("pre", { class: "tt-mail-body", text: n.text }),
      );
      return;
    }

    // Quick capture: "anotar" in a note by title.
    const title = h("input", { class: "input", placeholder: "Nota (ex.: jarvis, ideias)", style: "flex:0 0 34%" }) as HTMLInputElement;
    const text = h("input", { class: "input", placeholder: "O que anotar…", style: "flex:1" }) as HTMLInputElement;
    const save = async () => {
      if (!text.value.trim()) return;
      try {
        const p = await Bridge.notesWrite(title.value.trim() || "Ideias", text.value.trim());
        notes.flash = `Anotado em ${p}`;
        notes.hits = null;
      } catch (e) {
        notes.flash = String(e);
      }
      ctx.rerender();
    };
    text.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void save();
    });

    main.append(
      ctx.sectionHead("Notas · Obsidian"),
      searchBar("Buscar nas notas", notes.query, (q) => void searchNotes(q)),
      h("div", { class: "tt-searchbar", style: "margin-top:8px" }, title, text, h("button", { class: "btn", text: "Anotar", onclick: () => void save() })),
    );
    if (notes.flash) main.append(note(notes.flash));
    if (notes.loading) return void main.append(note("Lendo o vault…"));
    if (notes.error) return void main.append(note(notes.error, true));
    const hits = notes.hits ?? [];
    if (!hits.length) return void main.append(note(notes.query ? "Nenhuma nota com isso." : "Vault vazio."));
    main.append(
      h(
        "div",
        { class: "tt-list" },
        ...hits.map((n) =>
          h(
            "button",
            {
              class: "tt-row",
              onclick: async () => {
                try {
                  notes.open = { path: n.path, text: await Bridge.notesRead(n.path) };
                } catch (e) {
                  notes.error = String(e);
                }
                ctx.rerender();
              },
            },
            h("div", { class: "tt-row-top" }, h("span", { class: "tt-row-title", text: n.title }), h("span", { class: "tt-row-meta", text: fmtDate(n.modified) })),
            h("div", { class: "tt-row-sub", text: n.path.includes("/") ? n.path.slice(0, n.path.lastIndexOf("/")) : "" }),
            h("div", { class: "tt-row-snippet", text: n.snippet }),
          ),
        ),
      ),
    );
  }

  // ── Meu PC: áudio e internet ──────────────────────────────────────────────
  const pc = { audio: null as AudioState | null, audioErr: "", speed: "", testing: false, loadingAudio: false };

  async function loadAudio() {
    pc.loadingAudio = true;
    try {
      pc.audio = await Bridge.audioState();
      pc.audioErr = "";
    } catch (e) {
      pc.audioErr = String(e);
    }
    pc.loadingAudio = false;
    if (ctx.isOpen("pc")) ctx.rerender();
  }

  function renderPcExtras(container: HTMLElement) {
    if (!pc.audio && !pc.loadingAudio && !pc.audioErr) void loadAudio();
    const a = pc.audio;
    const out = h("div", { class: "tt-card" }, h("div", { class: "tt-card-title", text: "Saída de som" }));
    if (pc.audioErr) out.append(note(pc.audioErr, true));
    if (a) {
      out.append(
        h(
          "div",
          { class: "tt-outputs" },
          ...a.outputs.map((d) =>
            h("button", {
              class: `tt-chip ${d.default ? "on" : ""}`,
              text: d.name.replace(/\s*\(.*\)\s*$/, "") || d.name,
              title: d.name,
              onclick: async () => {
                await Bridge.audioSetOutput(d.id).catch(() => {});
                await loadAudio();
              },
            }),
          ),
        ),
      );
      const slider = h("input", { type: "range", min: "0", max: "100", value: String(a.volume), class: "tt-range" }) as HTMLInputElement;
      const label = h("span", { class: "tt-range-val", text: `${a.volume}%` });
      slider.addEventListener("input", () => (label.textContent = `${slider.value}%`));
      slider.addEventListener("change", () => void Bridge.setVolume(Number(slider.value)));
      out.append(
        h("div", { class: "tt-volume" }, h("span", { text: "Volume" }), slider, label),
        h(
          "div",
          { class: "tt-volume" },
          h("span", { text: a.micName ? `Microfone · ${a.micName.match(/\(([^)]*)\)/)?.[1]?.replace(/^\d+-\s*/, "").trim() || a.micName}` : "Microfone" }),
          a.micMuted == null
            ? h("span", { class: "tt-row-meta", text: "nenhum" })
            : h("button", {
                class: `btn btn-sm ${a.micMuted ? "btn-danger-soft" : ""}`,
                text: a.micMuted ? "Mudo — ligar" : "Ligado — mutar",
                onclick: async () => {
                  await Bridge.audioSetMicMute(!a.micMuted).catch(() => {});
                  await loadAudio();
                },
              }),
        ),
      );
    }
    const net = h(
      "div",
      { class: "tt-card" },
      h("div", { class: "tt-card-title", text: "Internet" }),
      h(
        "div",
        { class: "tt-volume" },
        h("span", { text: pc.speed || "Mede o download pela Cloudflare (~10 MB)." }),
        h("button", {
          class: "btn btn-sm",
          text: pc.testing ? "Medindo…" : "Testar velocidade",
          onclick: async () => {
            if (pc.testing) return;
            pc.testing = true;
            ctx.rerender();
            try {
              const mbps = await Bridge.netSpeedTest();
              pc.speed = `Download: ${mbps.toFixed(0)} Mbit/s`;
            } catch (e) {
              pc.speed = String(e).replace(/^Error:\s*/, "");
            }
            pc.testing = false;
            if (ctx.isOpen("pc")) ctx.rerender();
          },
        }),
      ),
    );
    container.append(h("div", { class: "tt-pc-grid" }, out, net));
  }

  // ── WhatsApp (WhatsApp Web hidden inside Jarvis, whatsapp.rs) ─────────────
  // Nothing is read until this tab is opened (the user asked for it).
  const wa = {
    state: null as WaState | null,
    chats: null as WaChat[] | null,
    thread: null as WaThread | null,
    opening: "",
    loading: false,
    error: "",
    draft: "",
    sending: false,
    timer: 0 as number,
    busy: false,
    /** How many chats are loaded ("Carregar mais" adds a page). */
    limit: 30,
    more: false,
    /** Messages loaded in the open chat ("anteriores" adds 30). */
    count: 30,
    keepTop: false,
    loadingMore: false,
    /** Voice note being recorded by WhatsApp. */
    recording: false,
    /** Voice note playing: its message id. */
    playing: "",
    /** Images fetched from the chat: message id → data: URL. */
    images: {} as Record<string, string>,
    imageLoading: "",
    imageFailed: new Set<string>(),
    /** WhatsApp's list filter: "all" | "unread" | "favorites" | "groups". */
    filter: "all",
    query: "",
    results: null as WaChat[] | null,
    searching: false,
    ticks: 0,
    /** The list was read fully once (the first open reads the top first). */
    loadedOnce: false,
    /** Chats where nothing older is left to load. */
    noOlder: new Set<string>(),
    /** Chats already opened: shown at once when opened again, then refreshed. */
    threads: new Map<string, WaThread>(),
  };

  const errText = (e: unknown) => String(e).replace(/^Error:\s*/, "");

  // ── Calls: WhatsApp's own call, driven from here (its window stays hidden) ──
  let call: WaCall | null = null;
  void Bridge.onWhatsappCall((c) => {
    const was = call?.state ?? "none";
    call = c.state === "none" ? null : c;
    if (c.state === "incoming" && was !== "incoming") {
      Sound.chime(0.9);
      ctx.toast?.(`📞 ${c.name || "Alguém"} está ligando no WhatsApp — atenda na aba WhatsApp do notch.`);
    }
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  });

  async function callAction(action: string) {
    try {
      const c = await Bridge.whatsappCallAction(action);
      call = c.state === "none" ? null : c;
    } catch (e) {
      wa.error = errText(e);
    }
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  function callBar(): HTMLElement | string {
    const c = call;
    if (!c) return "";
    const btn = (content: string | HTMLElement, action: string, cls = "btn btn-ghost btn-sm", title = typeof content === "string" ? content : "") =>
      h("button", { class: cls, title, "aria-label": title, onclick: () => void callAction(action) }, content);
    return h(
      "div",
      { class: `wa-call ${c.state}` },
      h("span", { class: "wa-call-dot" }),
      h(
        "div",
        { class: "wa-call-who" },
        h("div", { class: "wa-call-name", text: c.name || "Chamada do WhatsApp" }),
        h("div", { class: "wa-call-sub", text: c.state === "incoming" ? "Ligando para você…" : c.time || c.status || "Chamando…" }),
      ),
      c.state === "incoming"
        ? h("div", { class: "wa-call-actions" }, btn("Recusar", "decline", "btn btn-ghost btn-sm"), btn("Atender", "accept", "btn btn-sm wa-accept"))
        : h(
            "div",
            { class: "wa-call-actions" },
            btn(lineIcon(c.micOff ? "micOff" : "mic", 17), "mute", `wa-call-btn ${c.micOff ? "on" : ""}`, c.micOff ? "Ativar microfone" : "Silenciar microfone"),
            btn(lineIcon(c.camOff ? "videoOff" : "video", 17), "camera", `wa-call-btn ${c.camOff ? "" : "on"}`, c.camOff ? "Ligar câmera" : "Desligar câmera"),
            btn(lineIcon("window", 16), "show", "wa-call-btn", "Ver a janela da chamada"),
            btn(lineIcon("phoneOff", 17), "hangup", "wa-call-btn wa-hangup", "Desligar"),
          ),
    );
  }

  /** While the tab is on screen: QR / loading checks, then fresh chats. */
  function waTick() {
    if (!ctx.isOpen("whatsapp")) {
      window.clearInterval(wa.timer);
      wa.timer = 0;
      return;
    }
    if (wa.busy || wa.sending) return;
    wa.ticks++;
    if (wa.state?.state !== "ready") void waLoadState();
    // The open chat every tick; the list every third one (both columns show).
    else if (wa.thread && wa.ticks % 3 !== 0) void waOpen(wa.thread.title, true);
    else if (!wa.results) void waLoadChats(true);
  }

  async function waLoadState() {
    wa.busy = true;
    // The list as last read, at once (the real one comes right after).
    if (wa.chats == null) {
      const cached = await Bridge.whatsappChatsCached().catch(() => null);
      if (cached?.chats.length && wa.chats == null) {
        wa.chats = cached.chats;
        wa.more = cached.more;
        ctx.rerender();
      }
    }
    try {
      wa.state = await Bridge.whatsappState();
      wa.error = "";
      if (wa.state.state === "ready" && (wa.chats == null || !wa.loadedOnce)) {
        wa.busy = false;
        return void waLoadChats(wa.chats != null);
      }
    } catch (e) {
      wa.error = String(e).replace(/^Error:\s*/, "");
    }
    wa.busy = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waLoadChats(quiet = false) {
    wa.busy = true;
    if (!quiet) {
      wa.loading = true;
      ctx.rerender();
    }
    let same = false;
    try {
      // The periodic refresh reads only the top (fast); older chats already
      // loaded stay below it.
      // The first time: the chats on screen first (no scrolling), then the rest.
      const first = !wa.loadedOnce && wa.limit > 20;
      const page = await Bridge.whatsappChats(quiet || first ? 20 : wa.limit, wa.filter);
      if (first) {
        wa.loadedOnce = true;
        window.setTimeout(() => {
          if (!wa.busy) void waLoadAll();
        }, 50);
      }
      let next = page.chats;
      if (quiet && wa.chats && wa.chats.length > next.length) {
        const top = new Set(next.map((c) => c.title));
        next = [...next, ...wa.chats.filter((c) => !top.has(c.title))];
      } else {
        wa.more = page.more;
      }
      // Nothing new: no redraw (it cost a frame and the scroll position).
      same = quiet && JSON.stringify(next) === JSON.stringify(wa.chats) && !wa.error;
      wa.chats = next;
      wa.error = "";
    } catch (e) {
      wa.error = String(e).replace(/^Error:\s*/, "");
    }
    wa.loading = false;
    wa.busy = false;
    if (!same && ctx.isOpen("whatsapp")) ctx.rerender();
  }

  /** The rest of the first page of chats, quietly, after the top is shown. */
  async function waLoadAll() {
    wa.busy = true;
    try {
      const page = await Bridge.whatsappChats(wa.limit, wa.filter);
      if (page.chats.length) {
        wa.chats = page.chats;
        wa.more = page.more;
      }
    } catch {
      /* the top is there already */
    }
    wa.busy = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waMoreChats() {
    if (wa.loadingMore || wa.busy) return;
    wa.loadingMore = true;
    wa.busy = true;
    ctx.rerender();
    try {
      wa.limit += 40;
      const page = await Bridge.whatsappChats(wa.limit, wa.filter);
      wa.chats = page.chats;
      wa.more = page.more;
    } catch (e) {
      wa.error = errText(e);
    }
    wa.loadingMore = false;
    wa.busy = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waPlay(chat: string, id: string) {
    try {
      const st = await Bridge.whatsappPlay(chat, id);
      wa.playing = st === "playing" ? id : "";
    } catch (e) {
      wa.error = errText(e);
    }
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waShowImage(chat: string, id: string) {
    wa.imageLoading = id;
    ctx.rerender();
    try {
      wa.images[`${chat}:${id}`] = await Bridge.whatsappImage(chat, id);
    } catch {
      // Shown as "🖼 Imagem · Ver" instead; not retried by itself.
      wa.imageFailed.add(`${chat}:${id}`);
    }
    wa.imageLoading = "";
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waVoice(action: "start" | "send" | "cancel") {
    const t = wa.thread;
    if (!t) return;
    wa.busy = true;
    try {
      await Bridge.whatsappVoice(t.title, action);
      wa.recording = action === "start";
      wa.error = "";
      if (action === "send") window.setTimeout(() => void waOpen(t.title, true), 1500);
    } catch (e) {
      wa.recording = false;
      wa.error = errText(e);
    }
    wa.busy = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  /** One message bubble: text, or a control for its media. */
  function waBubble(t: WaThread, m: WaThread["messages"][number]) {
    const key = `${t.title}:${m.id}`;
    let body: HTMLElement | string;
    if (m.kind === "audio") {
      const playing = wa.playing === m.id;
      body = h(
        "div",
        { class: "wa-media" },
        h("button", { class: "wa-play", title: playing ? "Pausar" : "Ouvir", "aria-label": playing ? "Pausar" : "Ouvir", onclick: () => void waPlay(t.title, m.id) }, lineIcon(playing ? "pause" : "play", 15)),
        h("span", { text: `Áudio${m.duration ? ` · ${m.duration}` : ""}` }),
      );
    } else if (m.kind === "image") {
      const src = wa.images[key];
      body = src
        ? h(
            "div",
            {},
            h("img", { class: "wa-img", src, alt: "Imagem" }),
            h("button", {
              class: "link-btn",
              text: "Baixar",
              onclick: async () => {
                try {
                  const path = await Bridge.whatsappSaveImage(src, t.title);
                  wa.error = "";
                  ctx.toast?.(`Imagem salva em ${path}`);
                } catch (e) {
                  wa.error = errText(e);
                  ctx.rerender();
                }
              },
            }),
          )
        : h(
            "div",
            { class: "wa-media" },
            h("span", { class: "wa-media" }, lineIcon("image", 15), "Imagem"),
            h("button", { class: "link-btn", text: wa.imageLoading === m.id ? "carregando…" : "Ver", onclick: () => (wa.imageFailed.delete(key), void waShowImage(t.title, m.id)) }),
          );
    } else if (m.kind === "video") body = h("div", { class: "wa-media" }, lineIcon("film", 15), "Vídeo (abra no celular)");
    else if (m.kind === "document") body = h("div", { class: "wa-media" }, lineIcon("paperclip", 15), "Documento");
    else if (m.kind === "sticker") body = h("div", { class: "wa-media", text: "Figurinha" });
    else body = "";
    return h(
      "div",
      { class: `wa-msg ${m.mine ? "mine" : ""}` },
      !m.mine && m.from && m.from !== t.title ? h("div", { class: "wa-from", text: m.from }) : "",
      body,
      m.text ? h("div", { class: "wa-text", text: m.text }) : "",
      h("div", { class: "wa-when", text: (m.when.split(",")[0] || "").trim() }),
    );
  }

  async function waOpen(title: string, quiet = false) {
    wa.busy = true;
    if (!quiet) {
      wa.opening = title;
      // Another chat: its header right away, the messages when they come.
      if (wa.thread?.title !== title) {
        const known = (wa.results ?? wa.chats ?? []).find((c) => c.title === title);
        wa.thread = wa.threads.get(title) ?? { title, messages: [], avatar: known?.avatar ?? "" };
        wa.draft = "";
      }
      ctx.rerender();
    }
    try {
      if (!quiet && !wa.thread?.messages.length) wa.count = 30;
      const before = wa.thread?.title === title ? wa.thread.messages.length : 0;
      const t = await Bridge.whatsappThread(title, wa.count);
      // Asked for older ones and nothing more came: the start of the chat.
      if (!quiet && wa.keepTop && t.messages.length <= before) wa.noOlder.add(title);
      // Still on this chat (the user may have gone back meanwhile).
      if (quiet && wa.thread?.title !== title) return;
      // Nothing new and the user may be typing: leave the page alone.
      if (quiet && JSON.stringify(t) === JSON.stringify(wa.thread) && !wa.error) return;
      wa.thread = t;
      wa.threads.set(t.title, t);
      if (wa.threads.size > 30) wa.threads.delete(wa.threads.keys().next().value as string);
      wa.error = "";
    } catch (e) {
      wa.error = String(e).replace(/^Error:\s*/, "");
    } finally {
      wa.opening = "";
      wa.busy = false;
    }
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  async function waSend() {
    const t = wa.thread;
    const text = wa.draft.trim();
    if (!t || !text || wa.sending) return;
    wa.sending = true;
    ctx.rerender();
    try {
      await Bridge.whatsappSend(t.title, text);
      wa.draft = "";
      wa.sending = false;
      await waOpen(t.title, true);
    } catch (e) {
      wa.error = String(e).replace(/^Error:\s*/, "");
    }
    wa.sending = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  // ── The WhatsApp tab, laid out like WhatsApp: chats on the left, the open
  //    chat on the right. ──────────────────────────────────────────────────────
  const AVATAR_COLORS = ["#25d366", "#53bdeb", "#ffbc38", "#ff72a1", "#a78bfa", "#06cf9c", "#fb923c"];
  function avatar(name: string, src: string, size = 42): HTMLElement {
    if (src) return h("img", { class: "wa-avatar", src, alt: "", style: `width:${size}px;height:${size}px` });
    const letter = (name.replace(/[^\p{L}\p{N}]/gu, "")[0] || "?").toUpperCase();
    let n = 0;
    for (const ch of name) n = (n * 31 + ch.charCodeAt(0)) >>> 0;
    return h("div", {
      class: "wa-avatar wa-avatar-letter",
      text: letter,
      style: `width:${size}px;height:${size}px;background:${AVATAR_COLORS[n % AVATAR_COLORS.length]};font-size:${Math.round(size * 0.42)}px`,
    });
  }

  async function waSearch(q: string) {
    wa.query = q;
    if (!q.trim()) {
      wa.results = null;
      ctx.rerender();
      return;
    }
    wa.busy = true;
    wa.searching = true;
    ctx.rerender();
    try {
      wa.results = await Bridge.whatsappSearch(q);
    } catch (e) {
      wa.error = errText(e);
    }
    wa.searching = false;
    wa.busy = false;
    if (ctx.isOpen("whatsapp")) ctx.rerender();
  }

  function chatRow(c: WaChat) {
    const open = wa.thread?.title === c.title;
    return h(
      "button",
      { class: `wa-row ${open ? "open" : ""}`, onclick: () => void waOpen(c.title) },
      avatar(c.title, c.avatar),
      h(
        "div",
        { class: "wa-row-body" },
        h(
          "div",
          { class: "wa-row-top" },
          h("span", { class: "wa-row-name", text: c.title }),
          h("span", { class: `wa-row-time ${c.unread ? "unread" : ""}`, text: wa.opening === c.title ? "abrindo…" : c.time }),
        ),
        h(
          "div",
          { class: "wa-row-bottom" },
          h("span", { class: "wa-row-last", text: c.last }),
          c.unread ? h("span", { class: "wa-badge", text: String(c.unread) }) : "",
        ),
      ),
    );
  }

  function renderWhatsApp(main: HTMLElement) {
    if (!wa.timer) wa.timer = window.setInterval(waTick, 4000);
    if (wa.state == null && !wa.busy) void waLoadState();

    const st = wa.state?.state;
    if (st !== "ready") {
      main.append(callBar());
      if (wa.error) main.append(note(wa.error, true));
      if (st == null || st === "loading") {
        main.append(ctx.sectionHead("WhatsApp"), note("Carregando o WhatsApp…"));
        return;
      }
      main.append(
        ctx.sectionHead("WhatsApp"),
        h(
          "div",
          { class: "wa-link" },
          wa.state?.qr ? h("img", { class: "wa-qr", src: wa.state.qr, alt: "QR code do WhatsApp" }) : h("div", { class: "wa-qr wa-qr-empty", text: "Gerando QR code…" }),
          h(
            "div",
            { class: "wa-link-steps" },
            h("div", { class: "tt-card-title", text: "Conecte seu WhatsApp" }),
            h("ol", {}, h("li", { text: "Abra o WhatsApp no celular." }), h("li", { text: "Toque em ⋮ ou Configurações → Aparelhos conectados." }), h("li", { text: "Toque em Conectar um aparelho e aponte para este código." })),
            h("div", { class: "tt-hint", text: "A sessão fica só neste PC, dentro do Jarvis. Ele só lê ou envia quando você pede." }),
          ),
        ),
      );
      return;
    }

    main.classList.add("main-flush");
    if (wa.chats == null && !wa.loading && !wa.busy) void waLoadChats();

    // ── Left: search, filters, chats, the call bar ──
    const search = h("input", { class: "wa-search-input", placeholder: "Pesquisar conversa ou contato", value: wa.query }) as HTMLInputElement;
    search.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void waSearch(search.value);
      if (e.key === "Escape") void waSearch("");
    });
    const filters: [string, string][] = [
      ["all", "Tudo"],
      ["unread", "Não lidas"],
      ["favorites", "Favoritas"],
      ["groups", "Grupos"],
    ];
    const shown = wa.results ?? wa.chats ?? [];
    const list = h(
      "div",
      { class: "wa-chatlist" },
      ...(wa.searching
        ? [note("Procurando…")]
        : wa.loading && !wa.chats
          ? [note("Carregando conversas…")]
          : shown.length
            ? shown.map(chatRow)
            : [note(wa.results ? "Nada encontrado." : "Nenhuma conversa aqui.")]),
      !wa.results && (wa.more || wa.loadingMore)
        ? h("button", { class: "wa-more", text: wa.loadingMore ? "Carregando…" : "Carregar mais conversas", onclick: () => void waMoreChats() })
        : "",
    );
    const side = h(
      "aside",
      { class: "wa-side" },
      h(
        "div",
        { class: "wa-side-head" },
        h("span", { class: "wa-brand", text: "WhatsApp" }),
        h(
          "div",
          { style: "display:flex;gap:2px" },
          h("button", { class: "wa-icon-btn", title: "Abrir o WhatsApp completo numa janela", "aria-label": "Abrir o WhatsApp completo numa janela", onclick: () => void callAction("show") }, lineIcon("window", 16)),
          h("button", { class: "wa-icon-btn", title: "Atualizar", "aria-label": "Atualizar", onclick: () => void waLoadChats() }, lineIcon("refresh", 16)),
        ),
      ),
      h(
        "div",
        { class: "wa-search" },
        h("span", { class: "wa-search-icon" }, lineIcon("search", 15)),
        search,
        wa.query ? h("button", { class: "wa-icon-btn", title: "Limpar busca", "aria-label": "Limpar busca", onclick: () => void waSearch("") }, lineIcon("x", 15)) : "",
      ),
      h(
        "div",
        { class: "wa-filters" },
        ...filters.map(([id, label]) =>
          h("button", {
            class: `wa-filter ${wa.filter === id ? "on" : ""}`,
            text: label,
            onclick: () => {
              wa.filter = id;
              wa.results = null;
              wa.query = "";
              wa.limit = 30;
              void waLoadChats();
            },
          }),
        ),
      ),
      list,
      callBar(),
    );

    // ── Right: the open chat ──
    let pane: HTMLElement;
    const t = wa.thread;
    if (!t) {
      pane = h(
        "section",
        { class: "wa-pane wa-pane-empty" },
        h("div", { class: "wa-empty-title", text: "WhatsApp no Jarvis" }),
        h("div", { class: "wa-empty-sub", text: wa.opening ? "Abrindo a conversa…" : "Escolha uma conversa à esquerda. Ele só lê quando você abre ou pede." }),
      );
    } else {
      const input = h("input", { class: "wa-compose-input", placeholder: "Digite uma mensagem", value: wa.draft }) as HTMLInputElement;
      const sendOrMic = () => (wa.draft.trim() ? "send" : "mic");
      const action = h(
        "button",
        {
          class: "wa-round-btn",
          title: sendOrMic() === "send" ? "Enviar" : "Gravar áudio",
          "aria-label": sendOrMic() === "send" ? "Enviar" : "Gravar áudio",
          onclick: () => (wa.draft.trim() ? void waSend() : void waVoice("start")),
        },
        lineIcon(sendOrMic() === "send" ? "send" : "mic", 18),
      );
      input.addEventListener("input", () => {
        wa.draft = input.value;
        const label = sendOrMic() === "send" ? "Enviar" : "Gravar áudio";
        if (action.title !== label) {
          action.replaceChildren(lineIcon(sendOrMic() === "send" ? "send" : "mic", 18));
          action.title = label;
          action.setAttribute("aria-label", label);
        }
      });
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") void waSend();
      });
      const msgs = h(
        "div",
        { class: "wa-messages" },
        t.messages.length && !wa.noOlder.has(t.title)
          ? h("button", {
              class: "wa-older",
              text: wa.opening === t.title ? "Carregando…" : "Carregar mensagens anteriores",
              onclick: () => {
                wa.count += 30;
                wa.keepTop = true;
                void waOpen(t.title);
              },
            })
          : "",
        ...(t.messages.length
          ? t.messages.map((m) => waBubble(t, m))
          : [note(wa.opening === t.title ? "Carregando mensagens…" : "Sem mensagens recentes.")]),
      );
      pane = h(
        "section",
        { class: "wa-pane" },
        h(
          "div",
          { class: "wa-pane-head" },
          avatar(t.title, t.avatar, 36),
          h("div", { class: "wa-pane-name", text: t.title }),
          h("button", { class: "wa-icon-btn", title: "Chamada de vídeo", "aria-label": "Chamada de vídeo", onclick: () => void waCall(t.title, true) }, lineIcon("video", 18)),
          h("button", { class: "wa-icon-btn", title: "Ligar", "aria-label": "Ligar", onclick: () => void waCall(t.title, false) }, lineIcon("phone", 17)),
        ),
        msgs,
        wa.error ? h("div", { class: "wa-error", text: wa.error }) : "",
        wa.recording
          ? h(
              "div",
              { class: "wa-compose wa-recording" },
              h("span", { class: "wa-rec-dot" }),
              h("span", { style: "flex:1", text: "Gravando áudio… fale no microfone" }),
              h("button", { class: "btn btn-ghost btn-sm", text: "Cancelar", onclick: () => void waVoice("cancel") }),
              h("button", { class: "wa-round-btn", title: "Enviar áudio", "aria-label": "Enviar áudio", onclick: () => void waVoice("send") }, lineIcon("send", 18)),
            )
          : h("div", { class: "wa-compose" }, input, action),
      );
      queueMicrotask(() => {
        msgs.scrollTop = wa.keepTop ? 0 : msgs.scrollHeight;
        wa.keepTop = false;
        if (!wa.recording) input.focus();
      });
      // Images come in by themselves, one at a time.
      const pending = t.messages.find((m) => m.kind === "image" && !wa.images[`${t.title}:${m.id}`] && !wa.imageFailed.has(`${t.title}:${m.id}`));
      if (pending && !wa.imageLoading) window.setTimeout(() => void waShowImage(t.title, pending.id), 50);
    }

    main.append(h("div", { class: "wa-app" }, side, pane));
  }

  async function waCall(chat: string, video: boolean) {
    try {
      ctx.toast?.(await Bridge.whatsappCall(chat, video));
    } catch (e) {
      wa.error = errText(e);
      ctx.rerender();
    }
  }

  return { renderMail, renderWhatsApp, renderFiles, renderNotes, renderPcExtras, refreshAudio: loadAudio };
}

export function emptyInto(el: HTMLElement) {
  clear(el);
}
