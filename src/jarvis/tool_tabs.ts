// The newer notch tabs: E-mail (Gmail, loaded only when opened), Arquivos,
// Notas (Obsidian) and GitHub, plus the Áudio / Internet card of Meu PC.
// Each keeps its own state between renders; `rerender` redraws the panel.

import {
  Bridge,
  type AudioState,
  type FileHit,
  type GithubOverview,
  type GithubPr,
  type MailFull,
  type MailItem,
  type NoteHit,
} from "../core/bridge";
import { clear, h } from "../views/dom";

export interface ToolTabsCtx {
  rerender(): void;
  /** True while this tab is the one on screen (async results check it). */
  isOpen(tab: string): boolean;
  sectionHead(title: string, action?: { label: string; onclick: () => void }): HTMLElement;
  /** Sends a request to Jarvis (chat). */
  ask(prompt: string): void;
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

  // ── GitHub ────────────────────────────────────────────────────────────────
  const gh = { data: null as GithubOverview | null, loading: false, error: "", hasToken: null as boolean | null };

  async function loadGithub() {
    gh.loading = true;
    gh.error = "";
    ctx.rerender();
    gh.hasToken = await Bridge.secretPresent("github-token");
    if (gh.hasToken) {
      try {
        gh.data = await Bridge.githubOverview();
      } catch (e) {
        gh.error = String(e).replace(/^Error:\s*/, "");
      }
    }
    gh.loading = false;
    if (ctx.isOpen("github")) ctx.rerender();
  }

  const ciLabel: Record<string, [string, string]> = {
    success: ["ok", "CI ok"],
    failure: ["fail", "CI falhou"],
    pending: ["pending", "CI rodando"],
    unknown: ["none", "sem CI"],
  };

  function prRow(p: GithubPr, showReview: boolean): HTMLElement {
    const [cls, label] = ciLabel[p.ci] ?? ciLabel.unknown;
    const review = showReview
      ? { approved: "aprovado", changesRequested: "pediram mudanças", pending: "aguardando review" }[p.review] ?? ""
      : "";
    return h(
      "button",
      { class: "tt-row", onclick: () => void Bridge.openUrl(p.url) },
      h(
        "div",
        { class: "tt-row-top" },
        h("span", { class: "tt-row-title", text: p.title }),
        h("span", { class: `tt-ci ${cls}`, text: label }),
      ),
      h("div", { class: "tt-row-snippet", text: `${p.repo} #${p.number}${p.isDraft ? " · rascunho" : ""}${review ? ` · ${review}` : ""}` }),
    );
  }

  function renderGithub(main: HTMLElement) {
    if (gh.data == null && !gh.loading && !gh.error && gh.hasToken !== false) void loadGithub();
    main.append(ctx.sectionHead("GitHub", { label: "Atualizar", onclick: () => void loadGithub() }));
    if (gh.hasToken === false) {
      const input = h("input", { class: "input", type: "password", placeholder: "Token do GitHub (ghp_… ou github_pat_…)", style: "flex:1" }) as HTMLInputElement;
      main.append(
        h("div", { class: "tt-hint", text: "Crie um token em github.com/settings/tokens (escopos: repo, read:org) e cole aqui. Ele fica guardado no cofre do Windows." }),
        h(
          "div",
          { class: "tt-searchbar" },
          input,
          h("button", {
            class: "btn",
            text: "Conectar",
            onclick: async () => {
              if (!input.value.trim()) return;
              await Bridge.secretSet("github-token", input.value.trim());
              input.value = "";
              void loadGithub();
            },
          }),
        ),
      );
      return;
    }
    if (gh.loading) return void main.append(note("Carregando…"));
    if (gh.error) return void main.append(note(gh.error, true));
    const d = gh.data;
    if (!d) return;
    const failing = d.mainCI.filter((r) => r.ci === "failure");
    main.append(
      h("div", { class: "tt-hint", text: `Conectado como ${d.login}` }),
      ctx.sectionHead(`Reviews pedidas a você (${d.toReview.length})`),
      d.toReview.length ? h("div", { class: "tt-list" }, ...d.toReview.map((p) => prRow(p, false))) : note("Nenhuma review pendente."),
      ctx.sectionHead(`Seus PRs abertos (${d.myPRs.length})`),
      d.myPRs.length ? h("div", { class: "tt-list" }, ...d.myPRs.map((p) => prRow(p, true))) : note("Nenhum PR aberto."),
      ctx.sectionHead("CI dos seus repositórios"),
      failing.length
        ? h(
            "div",
            { class: "tt-list" },
            ...failing.map((r) =>
              h(
                "button",
                { class: "tt-row", onclick: () => void Bridge.openUrl(r.url) },
                h("div", { class: "tt-row-top" }, h("span", { class: "tt-row-title", text: r.repo }), h("span", { class: "tt-ci fail", text: "CI falhou" })),
                h("div", { class: "tt-row-snippet", text: `branch ${r.branch}` }),
              ),
            ),
          )
        : note(d.mainCI.length ? "Tudo verde nos repositórios principais." : "Sem CI nos repositórios recentes."),
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

  return { renderMail, renderFiles, renderNotes, renderGithub, renderPcExtras, refreshAudio: loadAudio };
}

export function emptyInto(el: HTMLElement) {
  clear(el);
}
