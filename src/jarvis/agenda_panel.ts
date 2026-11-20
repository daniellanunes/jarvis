// The Agenda tab: the user's Google Calendar inside Jarvis. A week at a time
// (all days shown, free ones too), events from every calendar ticked in Google
// Calendar, quick create, open in Google, delete with a second click to
// confirm. Same long-lived-element pattern as the Spotify panel.

import { Bridge, type CalEvent, type CalendarStatus } from "../core/bridge";
import { clear, h } from "../views/dom";

const I = {
  prev: "M15 18l-6-6 6-6",
  next: "M9 18l6-6-6-6",
  plus: "M12 5v14M5 12h14",
  ext: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
  video: "M3 7h12v10H3zM15 10l6-3v10l-6-3",
  pin: "M12 21s-7-6.5-7-12a7 7 0 0 1 14 0c0 5.5-7 12-7 12zM12 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4z",
};

function icon(d: string, size = 14): HTMLElement {
  const s = document.createElement("span");
  s.className = "mp-icon";
  s.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>`;
  return s;
}

const pad = (n: number) => String(n).padStart(2, "0");
/** Local date as YYYY-MM-DD. */
export function ymd(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
function addDays(d: Date, n: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
}
function hm(iso: string): string {
  const d = new Date(iso);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
/** The local day an event belongs to (all-day: its date; timed: its start). */
export function eventDay(e: CalEvent): string {
  return e.allDay ? e.start.slice(0, 10) : ymd(new Date(e.start));
}
export function eventSpan(e: CalEvent): string {
  return e.allDay ? "Dia todo" : `${hm(e.start)}–${hm(e.end)}`;
}
function errText(e: unknown): string {
  return String(e).replace(/^Error:\s*/, "");
}

const WEEKDAY = ["domingo", "segunda", "terça", "quarta", "quinta", "sexta", "sábado"];
const MONTH = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];

export interface AgendaHost {
  /** Today's events changed (Home's "Hoje" list). */
  onToday?(events: CalEvent[]): void;
}

export function createAgendaPanel(host: AgendaHost = {}) {
  const el = h("div", { class: "ag" });
  const listEl = h("div", { class: "ag-list" });
  const noticeEl = h("div", { class: "mp-notice" });
  let status: CalendarStatus | null = null;
  let start = new Date();
  let events: CalEvent[] = [];
  let loading = false;
  let formOpen = false;
  /** The account menu under the title (reconnect / disconnect). */
  let accountOpen = false;
  let busyLogin = false;
  let confirmKey = "";
  let visible = false;
  let poll: number | null = null;
  type View = "week" | "month" | "year";
  let view: View = "week";
  try {
    const saved = localStorage.getItem("jarvis_agenda_view");
    if (saved === "month" || saved === "year") view = saved;
  } catch {
    // Storage unavailable: start on the week.
  }

  /** First day and length of what the current view shows. */
  function range(): { from: Date; days: number } {
    if (view === "week") return { from: start, days: 7 };
    if (view === "month") {
      // Six full weeks, Sunday first, around the month of `start`.
      const first = new Date(start.getFullYear(), start.getMonth(), 1);
      return { from: addDays(first, -first.getDay()), days: 42 };
    }
    const jan1 = new Date(start.getFullYear(), 0, 1);
    const next = new Date(start.getFullYear() + 1, 0, 1);
    return { from: jan1, days: Math.round((next.getTime() - jan1.getTime()) / 86_400_000) };
  }

  /** Every local day an event covers (multi-day all-day events span several). */
  function daysOf(e: CalEvent): string[] {
    if (!e.allDay) return [eventDay(e)];
    const out: string[] = [];
    const [y, m, d] = e.start.slice(0, 10).split("-").map(Number);
    const endKey = e.end.slice(0, 10);
    let cur = new Date(y, m - 1, d);
    for (let i = 0; i < 400; i++) {
      const k = ymd(cur);
      if (i > 0 && k >= endKey) break;
      out.push(k);
      cur = addDays(cur, 1);
    }
    return out;
  }

  function byDay(): Map<string, CalEvent[]> {
    const map = new Map<string, CalEvent[]>();
    for (const e of events) {
      for (const k of daysOf(e)) {
        const list = map.get(k) ?? [];
        list.push(e);
        map.set(k, list);
      }
    }
    return map;
  }

  function setView(next: View, anchor?: Date) {
    view = next;
    if (anchor) start = anchor;
    try {
      localStorage.setItem("jarvis_agenda_view", view);
    } catch {
      // Not remembered: fine.
    }
    events = [];
    draw();
    void load();
  }

  function setNotice(text: string) {
    noticeEl.textContent = text;
    noticeEl.classList.toggle("on", Boolean(text));
    if (text) window.setTimeout(() => noticeEl.textContent === text && setNotice(""), 4500);
  }

  async function load() {
    if (!status?.connected) return;
    loading = true;
    drawList();
    try {
      const { from, days } = range();
      const asked = `${view}|${ymd(from)}`;
      const got = await Bridge.calendarEvents(ymd(from), days);
      // The user may have moved on while this was loading.
      if (asked !== `${view}|${ymd(range().from)}`) return;
      events = got;
      const today = ymd(new Date());
      if (ymd(from) <= today && today < ymd(addDays(from, days))) {
        host.onToday?.(events.filter((e) => daysOf(e).includes(today)));
      }
    } catch (e) {
      setNotice(errText(e));
      if (String(e).includes("expirou") || String(e).includes("não conectado")) {
        status = await Bridge.calendarStatus();
        draw();
        return;
      }
    }
    loading = false;
    drawList();
  }

  function dayLabel(d: Date): string {
    const today = ymd(new Date());
    const key = ymd(d);
    const prefix = key === today ? "Hoje · " : key === ymd(addDays(new Date(), 1)) ? "Amanhã · " : "";
    return `${prefix}${WEEKDAY[d.getDay()]}, ${d.getDate()} de ${MONTH[d.getMonth()]}`;
  }

  function eventRow(e: CalEvent): HTMLElement {
    const del = h("button", { class: `ag-act ${confirmKey === e.key ? "danger" : ""}`, title: "Apagar" },
      confirmKey === e.key ? h("span", { text: "Apagar?" }) : icon(I.trash, 13));
    del.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      if (confirmKey !== e.key) {
        confirmKey = e.key;
        drawList();
        return;
      }
      confirmKey = "";
      events = events.filter((x) => x.key !== e.key);
      drawList();
      try {
        await Bridge.calendarDelete(e.key);
        setNotice("Evento apagado.");
      } catch (err) {
        setNotice(errText(err));
        await load();
      }
    });
    const meta = [e.location, e.calendar].filter(Boolean).join(" · ");
    return h("div", { class: "ag-event" },
      h("span", { class: "ag-time", text: eventSpan(e) }),
      h("i", { class: "ag-bar", style: `background:${e.color || "var(--text-3)"}` }),
      h("span", { class: "ag-main" },
        h("span", { class: "ag-title", text: e.title }),
        meta ? h("span", { class: "ag-meta", text: meta }) : "",
      ),
      h("span", { class: "ag-actions" },
        e.meet ? h("button", { class: "ag-act", title: "Entrar na chamada", onclick: () => void Bridge.jarvisOpenTarget(e.meet) }, icon(I.video, 13)) : "",
        e.link ? h("button", { class: "ag-act", title: "Abrir no Google Agenda", onclick: () => void Bridge.jarvisOpenTarget(e.link) }, icon(I.ext, 13)) : "",
        del,
      ),
    );
  }

  const WEEK_HEAD = ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"];
  const MONTH_FULL = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];

  /** Six weeks, Sunday first; up to three events per day, then "+N". */
  function drawMonth() {
    listEl.className = "ag-month";
    const days = byDay();
    const today = ymd(new Date());
    const { from } = range();
    const grid = h("div", { class: "ag-mgrid" });
    for (const w of WEEK_HEAD) grid.append(h("div", { class: "ag-mhead", text: w }));
    for (let i = 0; i < 42; i++) {
      const d = addDays(from, i);
      const key = ymd(d);
      const list = days.get(key) ?? [];
      const cell = h("button", {
        class: `ag-mcell ${d.getMonth() !== start.getMonth() ? "out" : ""} ${key === today ? "today" : ""}`,
        title: `${dayLabel(d)} — ver a semana`,
        onclick: () => setView("week", d),
      }, h("span", { class: "ag-mnum", text: String(d.getDate()) }));
      for (const e of list.slice(0, 3)) {
        cell.append(h("span", { class: "ag-chip" },
          h("i", { style: `background:${e.color || "var(--text-3)"}` }),
          h("span", { text: e.allDay ? e.title : `${eventSpan(e).split("–")[0]} ${e.title}` }),
        ));
      }
      if (list.length > 3) cell.append(h("span", { class: "ag-more", text: `+${list.length - 3}` }));
      grid.append(cell);
    }
    listEl.append(grid);
  }

  /** Twelve small months; days with events are marked. */
  function drawYear() {
    listEl.className = "ag-year";
    const days = byDay();
    const today = ymd(new Date());
    const year = start.getFullYear();
    for (let m = 0; m < 12; m++) {
      const first = new Date(year, m, 1);
      const mini = h("div", { class: "ag-mini" },
        h("button", { class: "ag-mini-title", text: MONTH_FULL[m], title: "Ver o mês", onclick: () => setView("month", first) }),
      );
      const g = h("div", { class: "ag-mini-grid" });
      for (const w of WEEK_HEAD) g.append(h("span", { class: "ag-mini-head", text: w[0] }));
      for (let i = 0; i < first.getDay(); i++) g.append(h("span"));
      const len = new Date(year, m + 1, 0).getDate();
      for (let day = 1; day <= len; day++) {
        const d = new Date(year, m, day);
        const key = ymd(d);
        const n = days.get(key)?.length ?? 0;
        g.append(h("button", {
          class: `ag-mini-day ${n ? "busy" : ""} ${key === today ? "today" : ""}`,
          title: n ? `${dayLabel(d)} — ${n} ${n === 1 ? "evento" : "eventos"}` : dayLabel(d),
          text: String(day),
          onclick: () => setView("week", d),
        }));
      }
      mini.append(g);
      listEl.append(mini);
    }
  }

  function drawList() {
    clear(listEl);
    if (loading && !events.length) {
      listEl.className = "ag-list";
      listEl.append(h("div", { class: "mp-dim mp-pad", text: "Carregando agenda…" }));
      return;
    }
    if (view === "month") return drawMonth();
    if (view === "year") return drawYear();
    listEl.className = "ag-list";
    const days = byDay();
    for (let i = 0; i < 7; i++) {
      const d = addDays(start, i);
      const key = ymd(d);
      const dayEvents = days.get(key) ?? [];
      const isToday = key === ymd(new Date());
      listEl.append(h("div", { class: `ag-day ${isToday ? "today" : ""}` }, h("span", { text: dayLabel(d) })));
      if (!dayEvents.length) listEl.append(h("div", { class: "ag-free", text: "Livre" }));
      for (const e of dayEvents) listEl.append(eventRow(e));
    }
  }

  function drawForm(): HTMLElement {
    const now = new Date();
    const nextHour = new Date(now.getFullYear(), now.getMonth(), now.getDate(), now.getHours() + 1);
    const title = h("input", { class: "ag-input grow", placeholder: "Título do evento", spellcheck: "false" }) as HTMLInputElement;
    const date = h("input", { class: "ag-input", type: "date", value: ymd(nextHour) }) as HTMLInputElement;
    const from = h("input", { class: "ag-input", type: "time", value: `${pad(nextHour.getHours())}:00` }) as HTMLInputElement;
    const to = h("input", { class: "ag-input", type: "time", value: `${pad((nextHour.getHours() + 1) % 24)}:00` }) as HTMLInputElement;
    const allDay = h("input", { type: "checkbox" }) as HTMLInputElement;
    const where = h("input", { class: "ag-input grow", placeholder: "Local (opcional)", spellcheck: "false" }) as HTMLInputElement;
    // Google notifies on the phone; the Jarvis alerts on the PC.
    const remind = h(
      "select",
      { class: "ag-input", title: "Notificação", style: "color-scheme:dark" },
      h("option", { value: "", text: "Sem notificação" }),
      h("option", { value: "0", text: "Notificar na hora" }),
      h("option", { value: "5", text: "5 min antes" }),
      h("option", { value: "10", text: "10 min antes" }),
      h("option", { value: "30", text: "30 min antes" }),
      h("option", { value: "60", text: "1 hora antes" }),
      h("option", { value: "1440", text: "1 dia antes" }),
    ) as HTMLSelectElement;
    remind.value = "10";
    allDay.addEventListener("change", () => {
      from.disabled = to.disabled = allDay.checked;
    });
    const save = h("button", { class: "mp-primary", text: "Salvar" }) as HTMLButtonElement;
    save.addEventListener("click", async () => {
      if (!title.value.trim()) {
        setNotice("Dê um título ao evento.");
        title.focus();
        return;
      }
      save.disabled = true;
      try {
        await Bridge.calendarCreate({
          title: title.value.trim(),
          start: allDay.checked ? date.value : `${date.value}T${from.value}`,
          end: allDay.checked ? undefined : `${date.value}T${to.value}`,
          allDay: allDay.checked,
          location: where.value.trim() || undefined,
          remind: remind.value === "" ? null : Number(remind.value),
        });
        formOpen = false;
        setNotice("Evento criado.");
        draw();
        await load();
      } catch (e) {
        setNotice(errText(e));
        save.disabled = false;
      }
    });
    title.addEventListener("keydown", (e) => {
      if (e.key === "Enter") save.click();
    });
    window.setTimeout(() => title.focus(), 30);
    return h("div", { class: "ag-form" },
      h("div", { class: "ag-form-row" }, title),
      h("div", { class: "ag-form-row" },
        date, from, h("span", { class: "mp-dim", text: "até" }), to,
        h("label", { class: "ag-check" }, allDay, h("span", { text: "Dia todo" })),
        remind,
      ),
      h("div", { class: "ag-form-row" }, where,
        h("button", { class: "mp-btn-text", text: "Cancelar", onclick: () => { formOpen = false; draw(); } }),
        save,
      ),
    );
  }

  /** Switches the Google OAuth client (and so the Cloud project) Jarvis uses.
   *  Both values are stored in the OS keychain and win over the environment;
   *  the Gemini login shares this client. */
  function drawClientForm(): HTMLElement {
    const box = h("details", { class: "ag-client" }) as HTMLDetailsElement;
    if (!status?.configured) box.open = true;
    const id = h("input", { class: "mp-input", placeholder: "Client ID (…apps.googleusercontent.com)", spellcheck: "false" }) as HTMLInputElement;
    const secret = h("input", { class: "mp-input", type: "password", placeholder: "Client secret (GOCSPX-…)", spellcheck: "false" }) as HTMLInputElement;
    const save = h("button", { class: "mp-primary", text: "Usar este cliente" }) as HTMLButtonElement;
    save.addEventListener("click", async () => {
      const cid = id.value.trim();
      const sec = secret.value.trim();
      if (!/^\d+-[\w-]+\.apps\.googleusercontent\.com$/.test(cid)) {
        setNotice("Client ID inválido: ele termina em .apps.googleusercontent.com.");
        return;
      }
      if (!sec) {
        // Without it the old project's secret (environment) would be sent and Google would refuse.
        setNotice("Cole também o Client secret desse mesmo cliente.");
        return;
      }
      save.disabled = true;
      try {
        await Bridge.secretSet("gemini-oauth-client-id", cid);
        await Bridge.secretSet("gemini-oauth-client-secret", sec);
        // Tokens from the previous client cannot be refreshed with this one.
        await Bridge.calendarLogout();
        status = await Bridge.calendarStatus();
        setNotice(`Agora o Jarvis usa o projeto ${status?.project ?? ""}.`);
        draw();
      } catch (e) {
        setNotice(errText(e));
        save.disabled = false;
      }
    });
    box.append(
      h("summary", { text: "Usar o cliente OAuth de outro projeto do Google Cloud" }),
      h("ol", { class: "mp-steps" },
        h("li", { text: "No projeto certo, abra Google Auth Platform → Clientes → Criar cliente." }),
        h("li", { text: "Tipo de aplicativo: App para computador. Nome: Jarvis. Criar." }),
        h("li", { text: "Copie o ID do cliente e a Chave secreta do cliente e cole abaixo." }),
      ),
      h("div", { class: "ag-client-fields" }, id, secret, save),
    );
    return box;
  }

  function drawSetup() {
    const configured = status?.configured;
    const project = status?.project ?? "";
    // Links pinned to the project whose OAuth client Jarvis really uses:
    // test users added to any other project do not count.
    const inProject = (url: string) => (project ? `${url}?project=${project}` : url);
    el.append(
      h("div", { class: "mp-setup" },
        h("div", { class: "mp-h", text: "Conectar o Google Agenda" }),
        h("p", { class: "mp-dim", text: "Com a agenda conectada o Jarvis mostra seus compromissos e marca, muda ou cancela eventos por voz." }),
        project
          ? h("p", { class: "ag-project" },
              "O Jarvis usa o projeto do Google Cloud número ",
              h("code", { text: project }),
              ". Os passos abaixo valem só para ele: confira esse número no topo do Google Cloud (Painel → Número do projeto).")
          : "",
        h("ol", { class: "mp-steps" },
          configured
            ? ""
            : h("li", { text: "Configure primeiro o login do Google (Client ID OAuth do tipo App para computador) em Ajustes, o mesmo usado pelo Gemini." }),
          h("li", {}, "Ative a Google Calendar API nesse projeto: ",
            h("button", { class: "mp-link", text: "abrir no Google Cloud", onclick: () => void Bridge.jarvisOpenTarget(inProject("https://console.cloud.google.com/apis/library/calendar-json.googleapis.com")) })),
          h("li", {}, "Em Público-alvo → Usuários de teste, adicione seu e-mail: ",
            h("button", { class: "mp-link", text: "abrir", onclick: () => void Bridge.jarvisOpenTarget(inProject("https://console.cloud.google.com/auth/audience")) })),
          h("li", { text: "Clique em Conectar e autorize o Jarvis." }),
        ),
        h("p", { class: "mp-dim", text: "Enquanto o app do Google estiver em modo de teste, o Google pede para conectar de novo a cada 7 dias." }),
        drawClientForm(),
        h("div", { class: "mp-inline" },
          h("button", {
            class: "mp-primary",
            // While waiting the button stays usable: if Google showed an error
            // page (it never calls back then), a click starts a fresh attempt.
            text: busyLogin ? "Aguardando autorização… (clique para tentar de novo)" : "Conectar Google Agenda",
            disabled: !configured,
            onclick: async () => {
              busyLogin = true;
              draw();
              try {
                await Bridge.calendarLogin();
              } catch (e) {
                setNotice(errText(e));
              }
              busyLogin = false;
              status = await Bridge.calendarStatus();
              draw();
              await load();
            },
          }),
        ),
      ),
    );
  }

  /** Reconnect (e.g. to grant Gmail) or disconnect the Google account. */
  function drawAccount(): HTMLElement {
    const reconnect = async () => {
      busyLogin = true;
      draw();
      try {
        await Bridge.calendarLogout();
        await Bridge.calendarLogin();
        setNotice("Conta Google reconectada. Agenda e Gmail liberados.");
      } catch (e) {
        setNotice(errText(e));
      }
      busyLogin = false;
      accountOpen = false;
      status = await Bridge.calendarStatus();
      draw();
      await load();
    };
    return h(
      "div",
      { class: "ag-account-menu" },
      h("div", {
        class: "ag-account-text",
        text: "Agenda e Gmail usam esta conta. Para liberar o Gmail (ou trocar de conta), reconecte e marque todas as permissões na tela do Google.",
      }),
      h(
        "div",
        { class: "mp-inline" },
        h("button", {
          class: "mp-primary sm",
          text: busyLogin ? "Aguardando o Google… (clique para tentar de novo)" : "Reconectar conta",
          onclick: () => void reconnect(),
        }),
        h("button", {
          class: "mp-btn-text",
          text: "Desconectar",
          onclick: async () => {
            await Bridge.calendarLogout();
            accountOpen = false;
            status = await Bridge.calendarStatus();
            setNotice("Conta Google desconectada.");
            draw();
          },
        }),
      ),
    );
  }

  function draw() {
    clear(el);
    el.append(noticeEl);
    if (!status) {
      el.append(h("div", { class: "mp-dim mp-pad", text: "Carregando…" }));
      return;
    }
    if (!status.connected) return drawSetup();

    const end = addDays(start, 6);
    const label =
      view === "year"
        ? String(start.getFullYear())
        : view === "month"
          ? `${MONTH_FULL[start.getMonth()]} de ${start.getFullYear()}`
          : start.getMonth() === end.getMonth()
            ? `${start.getDate()}–${end.getDate()} de ${MONTH[end.getMonth()]}`
            : `${start.getDate()} de ${MONTH[start.getMonth()]} – ${end.getDate()} de ${MONTH[end.getMonth()]}`;
    const move = (dir: -1 | 0 | 1) => {
      if (dir === 0) start = new Date();
      else if (view === "week") start = addDays(start, 7 * dir);
      else if (view === "month") start = new Date(start.getFullYear(), start.getMonth() + dir, 1);
      else start = new Date(start.getFullYear() + dir, 0, 1);
      events = [];
      draw();
      void load();
    };
    const unit = view === "week" ? "Semana" : view === "month" ? "Mês" : "Ano";
    const seg = h("div", { class: "ag-seg", role: "tablist" },
      ...(["week", "month", "year"] as View[]).map((v) =>
        h("button", {
          class: view === v ? "on" : "",
          role: "tab",
          "aria-selected": view === v,
          text: v === "week" ? "Semana" : v === "month" ? "Mês" : "Ano",
          onclick: () => view !== v && setView(v),
        }),
      ),
    );
    el.append(
      h("div", { class: "ag-top" },
        h("div", { class: "ag-heading" },
          h("span", { class: "page-title", text: label.charAt(0).toUpperCase() + label.slice(1) }),
          h("button", {
            class: `ag-account ${accountOpen ? "open" : ""}`,
            title: "Conta Google: reconectar ou desconectar",
            text: `${status.email || "Conta Google"} ▾`,
            onclick: () => {
              accountOpen = !accountOpen;
              draw();
            },
          }),
        ),
        h("div", { class: "ag-nav" },
          seg,
          h("button", { class: "mp-btn", title: `${unit} anterior`, onclick: () => move(-1) }, icon(I.prev)),
          h("button", { class: "mp-btn-text", text: "Hoje", onclick: () => move(0) }),
          h("button", { class: "mp-btn", title: `Próximo ${unit.toLowerCase()}`, onclick: () => move(1) }, icon(I.next)),
          h("button", { class: "mp-primary sm", onclick: () => { formOpen = !formOpen; draw(); } }, icon(I.plus, 13), h("span", { text: "Novo evento" })),
        ),
      ),
      accountOpen ? drawAccount() : "",
      formOpen ? drawForm() : "",
      listEl,
    );
    drawList();
  }

  return {
    el,
    async show() {
      if (visible) return;
      visible = true;
      status = await Bridge.calendarStatus();
      draw();
      await load();
      poll = window.setInterval(() => void load(), 60_000);
    },
    hide() {
      if (!visible) return;
      visible = false;
      confirmKey = "";
      if (poll != null) window.clearInterval(poll);
      poll = null;
    },
    /** For Home: today's events, without opening the tab. */
    async today(): Promise<CalEvent[]> {
      const st = await Bridge.calendarStatus();
      if (!st?.connected) return [];
      const t = ymd(new Date());
      return (await Bridge.calendarEvents(t, 1)).filter((e) => eventDay(e) === t);
    },
  };
}
