// Spotify inside Jarvis, laid out like the Spotify desktop app: the library on
// the left, a home with shortcuts and recently played, playlist/album pages
// with a coloured header and a track table, search, and a player bar along
// the bottom (transport, seekable progress, volume, device). Everything goes
// through the Spotify Web API (spotify_api.rs). The panel is one long-lived
// element: the notch re-renders around it without losing state or scroll.

import {
  Bridge,
  type SpotifyApiStatus,
  type SpotifyDevice,
  type SpotifyItem,
  type SpotifyPlayer,
} from "../core/bridge";
import { clear, h } from "../views/dom";
import "./spotify.css";

const I = {
  play: "M7 4.5v15l12.5-7.5z",
  pause: "M7 4h3.5v16H7zM13.5 4H17v16h-3.5z",
  prev: "M6 5h2v14H6zM9.5 12L19 5v14z",
  next: "M16 5h2v14h-2zM14.5 12L5 19V5z",
  shuffle: "M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5",
  repeat: "M17 1l4 4-4 4M3 11V9a4 4 0 0 1 4-4h14M7 23l-4-4 4-4M21 13v2a4 4 0 0 1-4 4H3",
  volume: "M11 5L6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14",
  mute: "M11 5L6 9H2v6h4l5 4V5zM23 9l-6 6M17 9l6 6",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  home: "M3 10.5L12 3l9 7.5V21h-6v-6H9v6H3z",
  device: "M4 5h16v11H4zM8 20h8M12 16v4",
  copy: "M9 9h11v11H9zM5 15H4V4h11v1",
  ext: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
  heart: "M12 21s-8-5.2-8-11a4.5 4.5 0 0 1 8-2.8A4.5 4.5 0 0 1 20 10c0 5.8-8 11-8 11z",
  clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2",
  library: "M4 4v16M9 4v16M14 4l6 16",
};

function icon(d: string, size = 16, fill = false): HTMLElement {
  const s = document.createElement("span");
  s.className = "mp-icon";
  s.innerHTML = fill
    ? `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="currentColor"><path d="${d}"/></svg>`
    : `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>`;
  return s;
}

function fmt(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function errText(e: unknown): string {
  return String(e).replace(/^Error:\s*/, "");
}

const LIKED: SpotifyItem = {
  kind: "playlist",
  uri: "spotify:collection:tracks",
  id: "liked",
  name: "Músicas Curtidas",
  subtitle: "Playlist",
  image: "",
  durationMs: 0,
};

/** The average colour of a cover, darkened, for the page header gradient. */
const colorCache = new Map<string, string>();
function coverColor(url: string): Promise<string> {
  if (!url) return Promise.resolve("#3a3a44");
  const hit = colorCache.get(url);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      try {
        const c = document.createElement("canvas");
        c.width = c.height = 8;
        const ctx = c.getContext("2d")!;
        ctx.drawImage(img, 0, 0, 8, 8);
        const d = ctx.getImageData(0, 0, 8, 8).data;
        let r = 0, g = 0, b = 0;
        for (let i = 0; i < d.length; i += 4) {
          r += d[i];
          g += d[i + 1];
          b += d[i + 2];
        }
        const n = d.length / 4;
        // Keep it rich but dark enough for white text.
        const k = 0.75;
        const col = `rgb(${Math.round((r / n) * k)},${Math.round((g / n) * k)},${Math.round((b / n) * k)})`;
        colorCache.set(url, col);
        resolve(col);
      } catch {
        resolve("#3a3a44"); // the image server refused CORS
      }
    };
    img.onerror = () => resolve("#3a3a44");
    img.src = url;
  });
}

export interface MusicPanelHost {
  /** Saves the Spotify Client ID into the settings. */
  saveClientId(id: string): Promise<void>;
  /** Tells the notch that what is playing changed (pill, home card). */
  onTrackChange?(p: SpotifyPlayer | null): void;
}

type Page = { kind: "home" } | { kind: "collection"; item: SpotifyItem } | { kind: "search" };

export function createMusicPanel(host: MusicPanelHost) {
  const el = h("div", { class: "mp" });
  let status: SpotifyApiStatus | null = null;
  let player: SpotifyPlayer | null = null;
  let playerAt = 0;
  let playlists: SpotifyItem[] = [];
  let recent: SpotifyItem[] = [];
  let likedCount = 0;
  let devices: SpotifyDevice[] = [];
  let page: Page = { kind: "home" };
  let tracks: SpotifyItem[] = [];
  let results: SpotifyItem[] = [];
  let loadingPage = false;
  let query = "";
  let busyLogin = false;
  let showDevices = false;
  let visible = false;
  let pollTimer: number | null = null;
  let tickTimer: number | null = null;
  let searchTimer: number | null = null;
  let lastVolume = 50;

  const libEl = h("aside", { class: "sp-lib" });
  const viewEl = h("div", { class: "sp-view" });
  const barEl = h("footer", { class: "sp-bar" });
  const noticeEl = h("div", { class: "mp-notice" });

  function setNotice(text: string) {
    noticeEl.textContent = text;
    noticeEl.classList.toggle("on", Boolean(text));
    if (text) window.setTimeout(() => noticeEl.textContent === text && setNotice(""), 4500);
  }

  async function act(fn: () => Promise<unknown>, optimistic?: () => void) {
    optimistic?.();
    drawBar();
    markPlaying();
    try {
      await fn();
    } catch (e) {
      setNotice(errText(e));
    }
    window.setTimeout(() => void poll(), 400);
  }

  // ── Data ──────────────────────────────────────────────────────────────────
  async function poll() {
    if (!el.isConnected || !status?.connected) return;
    try {
      const p = await Bridge.spotifyPlayerState();
      const changed = (p?.uri ?? "") !== (player?.uri ?? "") || p?.playing !== player?.playing;
      player = p;
      playerAt = Date.now();
      if (p?.volume) lastVolume = p.volume;
      drawBar();
      if (changed) {
        host.onTrackChange?.(p);
        markPlaying();
      }
    } catch (e) {
      if (!String(e).includes("conexão")) setNotice(errText(e));
    }
  }

  async function refreshAll() {
    status = await Bridge.spotifyApiStatus();
    draw();
    if (!status?.connected) return;
    const [pl, p, rc, lc] = await Promise.allSettled([
      Bridge.spotifyMyPlaylists(),
      Bridge.spotifyPlayerState(),
      Bridge.spotifyRecent(),
      Bridge.spotifyLikedCount(),
    ]);
    if (pl.status === "fulfilled") playlists = pl.value;
    else setNotice(errText(pl.reason));
    if (p.status === "fulfilled") {
      player = p.value;
      playerAt = Date.now();
    }
    if (rc.status === "fulfilled") recent = rc.value;
    if (lc.status === "fulfilled") likedCount = lc.value;
    drawLib();
    drawBar();
    drawPage();
  }

  async function openCollection(item: SpotifyItem) {
    page = { kind: "collection", item };
    tracks = [];
    loadingPage = true;
    drawLib();
    drawPage();
    viewEl.scrollTop = 0;
    try {
      tracks = await Bridge.spotifyCollectionTracks(item.uri);
    } catch (e) {
      setNotice(errText(e));
    }
    loadingPage = false;
    if (page.kind === "collection" && page.item.uri === item.uri) drawPage();
  }

  function goHome() {
    page = { kind: "home" };
    query = "";
    const input = el.querySelector<HTMLInputElement>(".sp-search input");
    if (input) input.value = "";
    drawLib();
    drawPage();
    viewEl.scrollTop = 0;
  }

  async function runSearch(q: string) {
    if (!q.trim()) {
      if (page.kind === "search") goHome();
      return;
    }
    page = { kind: "search" };
    loadingPage = true;
    drawLib();
    drawPage();
    try {
      results = await Bridge.spotifySearch(q.trim());
    } catch (e) {
      setNotice(errText(e));
      results = [];
    }
    loadingPage = false;
    if (query === q && page.kind === "search") drawPage();
  }

  /** Plays an item: a collection from its start, or a track in its list. */
  function playItem(item: SpotifyItem, list?: SpotifyItem[], context?: string) {
    if (item.kind !== "track") {
      if (item.uri === LIKED.uri) {
        void (async () => {
          const liked = tracks.length && page.kind === "collection" && page.item.uri === LIKED.uri
            ? tracks
            : await Bridge.spotifyCollectionTracks(LIKED.uri).catch(() => []);
          if (liked.length) void act(() => Bridge.spotifyPlayList(liked.map((t) => t.uri), 0));
        })();
        return;
      }
      void act(() => Bridge.spotifyPlayUri(item.uri));
      return;
    }
    const optimistic = () => {
      if (player) player = { ...player, uri: item.uri, title: item.name, artist: item.subtitle.split(" · ")[0], image: item.image || player.image, progressMs: 0, durationMs: item.durationMs, playing: true };
      playerAt = Date.now();
    };
    if (context && context !== LIKED.uri && !context.startsWith("spotify:artist:")) {
      void act(() => Bridge.spotifyPlayUri(item.uri, context), optimistic);
    } else if (list?.length) {
      // No playable context (liked songs, recents, artist top tracks): play the list from here.
      void act(() => Bridge.spotifyPlayList(list.map((t) => t.uri), Math.max(0, list.indexOf(item))), optimistic);
    } else {
      void act(() => Bridge.spotifyPlayUri(item.uri), optimistic);
    }
  }

  // ── Library (left) ──────────────────────────────────────────────────────
  function cover(item: SpotifyItem, cls: string): HTMLElement {
    if (item.uri === LIKED.uri) return h("span", { class: `${cls} sp-liked-cover` }, icon(I.heart, 14, true));
    return h("img", { class: `${cls} ${item.kind === "artist" ? "round" : ""}`, src: item.image || "", alt: "", loading: "lazy" });
  }

  function drawLib() {
    clear(libEl);
    libEl.append(h("div", { class: "sp-lib-head" }, icon(I.library, 16), h("span", { text: "Sua Biblioteca" })));
    const list = h("div", { class: "sp-lib-list" });
    const all = [{ ...LIKED, subtitle: `Playlist · ${likedCount} músicas` }, ...playlists];
    for (const p of all) {
      const on = page.kind === "collection" && page.item.uri === p.uri;
      const playingHere = player?.contextUri && player.contextUri === p.uri;
      const row = h("button", { class: `sp-lib-row ${on ? "on" : ""} ${playingHere ? "playing" : ""}`, title: p.name },
        cover(p, "sp-lib-cover"),
        h("span", { class: "sp-lib-text" },
          h("span", { class: "sp-lib-name", text: p.name }),
          h("span", { class: "sp-lib-sub", text: p.uri === LIKED.uri ? p.subtitle : `Playlist · ${p.subtitle.split(" · ")[0]}` }),
        ),
      );
      row.addEventListener("click", () => void openCollection(p));
      list.append(row);
    }
    libEl.append(list);
  }

  // ── Pages (centre) ──────────────────────────────────────────────────────
  function topBar(): HTMLElement {
    const search = h("input", { placeholder: "O que você quer ouvir?", spellcheck: "false", value: query }) as HTMLInputElement;
    search.addEventListener("input", () => {
      query = search.value;
      if (searchTimer) window.clearTimeout(searchTimer);
      searchTimer = window.setTimeout(() => void runSearch(query), 280);
    });
    search.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && search.value) {
        e.stopPropagation();
        goHome();
      }
    });
    return h("div", { class: "sp-top" },
      h("button", { class: `sp-round ${page.kind === "home" ? "on" : ""}`, title: "Início", onclick: goHome }, icon(I.home, 17, page.kind === "home")),
      h("label", { class: "sp-search" }, icon(I.search, 16), search),
      h("button", { class: "sp-round", title: "Abrir o app do Spotify", onclick: () => void Bridge.jarvisOpenTarget("spotify") }, icon(I.ext, 15)),
    );
  }

  function playCircle(onclick: () => void, size: "lg" | "sm" = "sm"): HTMLElement {
    const b = h("button", { class: `sp-play ${size}`, title: "Tocar" }, icon(I.play, size === "lg" ? 20 : 16, true));
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      onclick();
    });
    return b;
  }

  function drawHome(body: HTMLElement) {
    const hour = new Date().getHours();
    body.append(h("div", { class: "sp-greet", text: hour < 12 ? "Bom dia" : hour < 18 ? "Boa tarde" : "Boa noite" }));
    const grid = h("div", { class: "sp-shortcuts" });
    for (const p of [{ ...LIKED }, ...playlists.slice(0, 7)]) {
      const tile = h("div", { class: "sp-tile", role: "button", tabindex: 0, title: p.name },
        cover(p, "sp-tile-cover"),
        h("span", { class: "sp-tile-name", text: p.name }),
        playCircle(() => playItem(p)),
      );
      tile.addEventListener("click", () => void openCollection(p));
      grid.append(tile);
    }
    body.append(grid);

    if (recent.length) {
      body.append(h("div", { class: "sp-section" }, h("span", { text: "Tocadas recentemente" })));
      const row = h("div", { class: "sp-cards" });
      for (const t of recent.slice(0, 12)) {
        const card = h("div", { class: "sp-card", role: "button", tabindex: 0, title: `${t.name} — ${t.subtitle}` },
          h("span", { class: "sp-card-art" }, h("img", { src: t.image || "", alt: "", loading: "lazy" }), playCircle(() => playItem(t, recent))),
          h("span", { class: "sp-card-title", text: t.name }),
          h("span", { class: "sp-card-sub", text: t.subtitle.split(" · ")[0] }),
        );
        card.addEventListener("click", () => playItem(t, recent));
        row.append(card);
      }
      body.append(row);
    }
  }

  function trackTable(list: SpotifyItem[], context?: string): HTMLElement {
    const table = h("div", { class: "sp-table" },
      h("div", { class: "sp-tr head" },
        h("span", { class: "sp-td idx", text: "#" }),
        h("span", { class: "sp-td title", text: "Título" }),
        h("span", { class: "sp-td album", text: "Álbum" }),
        h("span", { class: "sp-td dur" }, icon(I.clock, 14)),
      ),
    );
    list.forEach((t, i) => {
      const [artist, album] = t.subtitle.split(" · ");
      const row = h("div", { class: "sp-tr", role: "button", tabindex: 0, "data-uri": t.uri, title: `Tocar ${t.name}` },
        h("span", { class: "sp-td idx" }, h("span", { class: "n", text: String(i + 1) }), h("span", { class: "p" }, icon(I.play, 12, true))),
        h("span", { class: "sp-td title" },
          t.image ? h("img", { src: t.image, alt: "", loading: "lazy" }) : "",
          h("span", { class: "sp-tt" }, h("span", { class: "sp-tt-name", text: t.name }), h("span", { class: "sp-tt-artist", text: artist || "" })),
        ),
        h("span", { class: "sp-td album", text: album || "" }),
        h("span", { class: "sp-td dur", text: fmt(t.durationMs) }),
      );
      row.addEventListener("click", () => playItem(t, list, context));
      row.addEventListener("dblclick", () => playItem(t, list, context));
      table.append(row);
    });
    return table;
  }

  function drawCollection(body: HTMLElement, item: SpotifyItem) {
    const kind = item.uri === LIKED.uri ? "Playlist" : item.kind === "album" ? "Álbum" : item.kind === "artist" ? "Artista" : "Playlist";
    const sub = item.uri === LIKED.uri ? `${likedCount} músicas` : item.subtitle;
    const head = h("div", { class: "sp-hero" },
      cover(item, "sp-hero-cover"),
      h("div", { class: "sp-hero-meta" },
        h("span", { class: "sp-hero-kind", text: kind }),
        h("span", { class: `sp-hero-title ${item.name.length > 22 ? "long" : ""}`, text: item.name }),
        h("span", { class: "sp-hero-sub", text: sub }),
      ),
    );
    // Header tinted with the cover's colour (purple for liked songs), like Spotify.
    const tint = item.uri === LIKED.uri ? Promise.resolve("rgb(80,56,160)") : coverColor(item.image);
    void tint.then((c) => {
      head.style.setProperty("--tint", c);
      actions.style.setProperty("--tint", c);
    });
    const shuffle = h("button", { class: `sp-icon-btn ${player?.shuffle ? "on" : ""}`, title: "Aleatório" }, icon(I.shuffle, 20));
    shuffle.addEventListener("click", () => {
      const on = !player?.shuffle;
      void act(() => Bridge.spotifyApiControl("shuffle", on ? 1 : 0), () => player && (player = { ...player, shuffle: on }));
      shuffle.classList.toggle("on", on);
    });
    const actions = h("div", { class: "sp-actions" },
      playCircle(() => playItem(item, tracks, item.uri), "lg"),
      shuffle,
    );
    body.append(head, actions);
    if (loadingPage && !tracks.length) {
      body.append(h("div", { class: "sp-empty", text: "Carregando…" }));
    } else if (!tracks.length) {
      body.append(h("div", { class: "sp-empty", text: "Nenhuma música aqui (ou o Spotify não liberou o conteúdo desta playlist)." }));
    } else {
      body.append(trackTable(tracks, item.kind === "artist" ? undefined : item.uri));
    }
  }

  function drawSearch(body: HTMLElement) {
    if (loadingPage && !results.length) {
      body.append(h("div", { class: "sp-empty", text: "Buscando…" }));
      return;
    }
    const songs = results.filter((r) => r.kind === "track");
    const others = results.filter((r) => r.kind !== "track");
    if (!results.length) {
      body.append(h("div", { class: "sp-empty", text: `Nada encontrado para "${query}".` }));
      return;
    }
    if (songs.length) {
      body.append(h("div", { class: "sp-section" }, h("span", { text: "Músicas" })), trackTable(songs.slice(0, 6)));
    }
    if (others.length) {
      body.append(h("div", { class: "sp-section" }, h("span", { text: "Álbuns, playlists e artistas" })));
      const row = h("div", { class: "sp-cards wrap" });
      for (const it of others) {
        const card = h("div", { class: "sp-card", role: "button", tabindex: 0, title: it.name },
          h("span", { class: `sp-card-art ${it.kind === "artist" ? "round" : ""}` }, h("img", { src: it.image || "", alt: "", loading: "lazy" }), playCircle(() => playItem(it))),
          h("span", { class: "sp-card-title", text: it.name }),
          h("span", { class: "sp-card-sub", text: it.kind === "album" ? `Álbum · ${it.subtitle}` : it.kind === "artist" ? "Artista" : "Playlist" }),
        );
        card.addEventListener("click", () => void openCollection(it));
        row.append(card);
      }
      body.append(row);
    }
  }

  function drawPage() {
    const keepFocus = document.activeElement?.closest(".sp-search") != null;
    clear(viewEl);
    viewEl.append(topBar());
    const body = h("div", { class: "sp-body" });
    if (page.kind === "home") drawHome(body);
    else if (page.kind === "collection") drawCollection(body, page.item);
    else drawSearch(body);
    viewEl.append(body);
    if (keepFocus) {
      const input = viewEl.querySelector<HTMLInputElement>(".sp-search input");
      input?.focus();
      input?.setSelectionRange(input.value.length, input.value.length);
    }
    markPlaying();
  }

  function markPlaying() {
    for (const r of viewEl.querySelectorAll<HTMLElement>(".sp-tr[data-uri]")) {
      const on = r.dataset.uri === player?.uri;
      r.classList.toggle("playing", on);
      r.classList.toggle("paused", on && !player?.playing);
    }
  }

  // ── Player bar (bottom) ─────────────────────────────────────────────────
  function progressNow(): number {
    if (!player) return 0;
    const elapsed = player.playing ? Date.now() - playerAt : 0;
    return Math.min(player.durationMs, player.progressMs + elapsed);
  }

  function slider(value: number, max: number, cls: string, onCommit: (v: number) => void, live = false): HTMLElement {
    const wrap = h("div", { class: `sp-slider ${cls}` }, h("i", { style: `width:${max ? (value / max) * 100 : 0}%` }));
    const at = (e: MouseEvent) => {
      const r = wrap.getBoundingClientRect();
      return Math.min(max, Math.max(0, ((e.clientX - r.left) / r.width) * max));
    };
    wrap.addEventListener("mousedown", (e) => {
      const fill = wrap.querySelector("i") as HTMLElement;
      const move = (ev: MouseEvent) => {
        fill.style.width = `${(at(ev) / max) * 100}%`;
        if (live) onCommit(at(ev));
      };
      const up = (ev: MouseEvent) => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
        onCommit(at(ev));
      };
      move(e);
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    });
    return wrap;
  }

  function drawBar() {
    clear(barEl);
    const p = player;
    if (!p || !p.title) {
      barEl.append(h("div", { class: "sp-bar-empty", text: "Nada tocando. Escolha algo acima ou peça ao Jarvis: “toca Dawn FM”." }));
      return;
    }
    const pos = progressNow();
    let volTimer: number | null = null;
    const vol = slider(p.volume ?? lastVolume, 100, "vol", (v) => {
      if (volTimer) window.clearTimeout(volTimer);
      volTimer = window.setTimeout(() => void Bridge.spotifyApiControl("volume", Math.round(v)).catch((e) => setNotice(errText(e))), 120);
    }, true);
    const muted = (p.volume ?? 1) === 0;
    const repeatNext = p.repeat === "off" ? 1 : p.repeat === "context" ? 2 : 0;
    const devBtn = h("button", { class: `sp-icon-btn sm ${showDevices ? "on" : ""}`, title: `Tocando em ${p.device || "este dispositivo"}` }, icon(I.device, 16));
    devBtn.addEventListener("click", async () => {
      showDevices = !showDevices;
      if (showDevices) devices = await Bridge.spotifyDevices().catch((e) => (setNotice(errText(e)), []));
      drawBar();
    });

    barEl.append(
      h("div", { class: "sp-bar-left" },
        p.image ? h("img", { class: "sp-bar-cover", src: p.image, alt: "" }) : h("span", { class: "sp-bar-cover" }),
        h("div", { class: "sp-bar-meta" },
          h("span", { class: "sp-bar-title", text: p.title }),
          h("span", { class: "sp-bar-artist", text: p.artist }),
        ),
      ),
      h("div", { class: "sp-bar-center" },
        h("div", { class: "sp-bar-buttons" },
          h("button", {
            class: `sp-icon-btn sm ${p.shuffle ? "on" : ""}`, title: "Aleatório",
            onclick: () => void act(() => Bridge.spotifyApiControl("shuffle", p.shuffle ? 0 : 1), () => (player = { ...p, shuffle: !p.shuffle })),
          }, icon(I.shuffle, 15)),
          h("button", { class: "sp-icon-btn sm", title: "Anterior", onclick: () => void act(() => Bridge.spotifyApiControl("previous")) }, icon(I.prev, 15, true)),
          h("button", {
            class: "sp-play-bar", title: p.playing ? "Pausar" : "Tocar",
            onclick: () => void act(() => Bridge.spotifyApiControl(p.playing ? "pause" : "play"), () => {
              player = { ...p, playing: !p.playing, progressMs: progressNow() };
              playerAt = Date.now();
            }),
          }, icon(p.playing ? I.pause : I.play, 15, true)),
          h("button", { class: "sp-icon-btn sm", title: "Próxima", onclick: () => void act(() => Bridge.spotifyApiControl("next")) }, icon(I.next, 15, true)),
          h("button", {
            class: `sp-icon-btn sm ${p.repeat !== "off" ? "on" : ""}`, title: p.repeat === "track" ? "Repetir esta música" : "Repetir",
            onclick: () => void act(() => Bridge.spotifyApiControl("repeat", repeatNext)),
          }, icon(I.repeat, 15), p.repeat === "track" ? h("b", { class: "sp-one", text: "1" }) : ""),
        ),
        h("div", { class: "sp-progress" },
          h("span", { class: "sp-time now", text: fmt(pos) }),
          slider(pos, p.durationMs, "seek", (ms) => {
            void act(() => Bridge.spotifyApiControl("seek", Math.round(ms)), () => {
              player = { ...p, progressMs: Math.round(ms) };
              playerAt = Date.now();
            });
          }),
          h("span", { class: "sp-time", text: fmt(p.durationMs) }),
        ),
      ),
      h("div", { class: "sp-bar-right" },
        devBtn,
        h("button", {
          class: "sp-icon-btn sm", title: muted ? "Ativar som" : "Sem som",
          onclick: () => void act(() => Bridge.spotifyApiControl("volume", muted ? lastVolume || 50 : 0), () => {
            if (!muted && p.volume) lastVolume = p.volume;
            player = { ...p, volume: muted ? lastVolume || 50 : 0 };
          }),
        }, icon(muted ? I.mute : I.volume, 15)),
        vol,
      ),
    );

    if (showDevices) {
      const menu = h("div", { class: "sp-devices" }, h("div", { class: "sp-devices-head", text: "Conectar a um dispositivo" }));
      if (!devices.length) menu.append(h("div", { class: "sp-empty", text: "Nenhum dispositivo. Abra o Spotify em algum aparelho." }));
      for (const d of devices) {
        menu.append(h("button", {
          class: `sp-device ${d.active ? "on" : ""}`,
          onclick: () => {
            showDevices = false;
            void act(() => Bridge.spotifyApiControl("transfer", 0, d.id));
          },
        }, icon(I.device, 15), h("span", { class: "grow", text: d.name }), h("span", { class: "dim", text: d.kind === "Computer" ? "Computador" : d.kind === "Smartphone" ? "Celular" : d.kind })));
      }
      barEl.append(menu);
    }
  }

  // ── Setup / login ───────────────────────────────────────────────────────
  function drawSetup() {
    const idIn = h("input", { class: "mp-input", placeholder: "Cole aqui o Client ID", spellcheck: "false" }) as HTMLInputElement;
    const copyBtn = h("button", { class: "mp-btn-text", title: "Copiar" }, icon(I.copy, 13), h("span", { text: "Copiar" }));
    copyBtn.addEventListener("click", () => {
      void navigator.clipboard?.writeText(status?.redirectUri ?? "");
      setNotice("Endereço copiado.");
    });
    el.append(
      h("div", { class: "mp-setup" },
        h("div", { class: "mp-h", text: "Conectar o Spotify ao Jarvis" }),
        h("p", { class: "mp-dim", text: "Uma vez só. Com isso o Jarvis toca exatamente a música, álbum ou playlist que você pedir." }),
        h("ol", { class: "mp-steps" },
          h("li", {}, "Abra ", h("button", { class: "mp-link", text: "developer.spotify.com/dashboard", onclick: () => void Bridge.jarvisOpenTarget("https://developer.spotify.com/dashboard") }), " e entre com sua conta."),
          h("li", { text: "Clique em Create app. Nome e descrição: Jarvis. Marque Web API." }),
          h("li", {}, "Em Redirect URIs, adicione: ", h("code", { text: status?.redirectUri ?? "" }), copyBtn),
          h("li", { text: "Salve, abra o app criado e copie o Client ID." }),
        ),
        h("div", { class: "mp-inline" },
          idIn,
          h("button", {
            class: "mp-primary", text: "Salvar",
            onclick: async () => {
              const v = idIn.value.trim();
              if (!/^[0-9a-f]{32}$/i.test(v)) {
                setNotice("O Client ID tem 32 caracteres (letras e números).");
                return;
              }
              await host.saveClientId(v);
              await refreshAll();
            },
          }),
        ),
      ),
    );
  }

  function drawLogin() {
    el.append(
      h("div", { class: "mp-setup" },
        h("div", { class: "mp-h", text: "Entrar no Spotify" }),
        h("p", { class: "mp-dim", text: "Vamos abrir a página do Spotify no navegador para você autorizar o Jarvis. Depois é só voltar aqui." }),
        h("div", { class: "mp-inline" },
          h("button", {
            class: "mp-primary", text: busyLogin ? "Aguardando autorização… (clique para tentar de novo)" : "Conectar com Spotify",
            onclick: async () => {
              busyLogin = true;
              draw();
              try {
                await Bridge.spotifyLogin();
              } catch (e) {
                setNotice(errText(e));
              }
              busyLogin = false;
              await refreshAll();
            },
          }),
          h("button", { class: "mp-btn-text", text: "Trocar Client ID", onclick: async () => { await host.saveClientId(""); await refreshAll(); } }),
        ),
      ),
    );
  }

  function draw() {
    clear(el);
    el.classList.remove("sp");
    el.append(noticeEl);
    if (!status) {
      el.append(h("div", { class: "mp-dim mp-pad", text: "Carregando Spotify…" }));
      return;
    }
    if (!status.clientIdSet) return drawSetup();
    if (!status.connected) return drawLogin();
    el.classList.add("sp");
    el.append(libEl, viewEl, barEl);
    drawLib();
    drawPage();
    drawBar();
  }

  return {
    el,
    /** Called on every render of the music tab; only the first one loads. */
    async show() {
      if (visible) return;
      visible = true;
      if (pollTimer == null) pollTimer = window.setInterval(() => void poll(), 3000);
      if (tickTimer == null) {
        tickTimer = window.setInterval(() => {
          if (!el.isConnected || !player?.playing) return;
          const pos = progressNow();
          const fill = barEl.querySelector<HTMLElement>(".sp-slider.seek i");
          const time = barEl.querySelector<HTMLElement>(".sp-time.now");
          if (fill && player.durationMs && !barEl.matches(":active")) fill.style.width = `${(pos / player.durationMs) * 100}%`;
          if (time) time.textContent = fmt(pos);
          if (pos >= player.durationMs && player.durationMs) void poll();
        }, 500);
      }
      if (!status) draw();
      await refreshAll();
    },
    hide() {
      if (!visible) return;
      visible = false;
      if (pollTimer != null) window.clearInterval(pollTimer);
      if (tickTimer != null) window.clearInterval(tickTimer);
      pollTimer = tickTimer = null;
      showDevices = false;
    },
  };
}
