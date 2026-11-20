import "./dashboard.css";
import {
  Bridge,
  onEvent,
  type JarvisBrainStatus,
  type JarvisData,
  type MemoryNode,
  type PcStats,
} from "../core/bridge";
import { Sound } from "../core/sound";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { Voice } from "../jarvis/voice";
import { clear, h } from "../views/dom";
import { renderMemoryGraph } from "./memory_graph";

type TabId =
  | "home"
  | "chat"
  | "memory"
  | "tasks"
  | "reminders"
  | "habits"
  | "timers"
  | "music"
  | "clipboard"
  | "routines"
  | "pc"
  | "settings";

interface ChatTurn {
  role: "user" | "assistant";
  content: string;
  meta?: string;
}

interface ActiveTimer {
  id: string;
  label: string;
  remainingSec: number;
  running: boolean;
}

let activeTab: TabId = "home";
let settings: Settings = { ...DEFAULT_SETTINGS };
let brainStatus: JarvisBrainStatus | null = null;
let data: JarvisData = {
  userName: "Chefe",
  accentColor: "#6366f1",
  memories: [],
  tasks: [],
  reminders: [],
  habits: [],
  routines: [],
};

const chatLog: ChatTurn[] = [];
const clipboardHistory: string[] = [];
const timers: ActiveTimer[] = [];
let lastClip = "";
let sending = false;
let cleanupGraph: (() => void) | null = null;
let selectedMemory: MemoryNode | null = null;
let memoryFilter = "";
let pcStats: PcStats | null = null;
let pcScanText = "";

const root = document.getElementById("dashboard-root")!;

async function saveData() {
  await Bridge.jarvisDataSave(data);
}

async function saveSettings() {
  await Bridge.saveSettings(settings);
}

async function askJarvis(prompt: string, withScreen = false) {
  const q = prompt.trim();
  if (!q || sending) return;
  sending = true;
  chatLog.push({ role: "user", content: withScreen ? `[Tela] ${q}` : q });
  render();

  try {
    let img: string | undefined;
    if (withScreen) {
      img = await Bridge.jarvisCaptureScreen();
    }
    const history = chatLog.slice(-10).map((m) => ({ role: m.role, content: m.content }));
    const reply = await Bridge.jarvisAsk(q, history, undefined, img);
    const actionsNote = reply.actions.length ? ` · ${reply.actions.join(", ")}` : "";
    chatLog.push({
      role: "assistant",
      content: reply.text,
      meta: `${reply.providerUsed}${actionsNote}`,
    });
    Sound.play("finish");
    Voice.speak(reply.text);
    // Reload data in case Jarvis saved a memory or task via [[ACTION:...]]
    const fresh = await Bridge.jarvisDataLoad();
    if (fresh) data = fresh;
  } catch (err) {
    chatLog.push({
      role: "assistant",
      content: String(err).replace(/^Error:\s*/, ""),
      meta: "Erro",
    });
    Sound.play("error");
  } finally {
    sending = false;
    render();
  }
}

// ── Tabs ──────────────────────────────────────────────────────────────────────

function renderHome(main: HTMLElement) {
  const input = h("input", {
    class: "input",
    style: "flex:1",
    placeholder: "Escreva, fale ou diga 'Ei, Jarvis'…",
  }) as HTMLInputElement;

  const sendBtn = h("button", {
    class: "btn",
    text: sending ? "Pensando…" : "Enviar",
    onclick: () => {
      const val = input.value;
      if (!val.trim()) return;
      activeTab = "chat";
      void askJarvis(val);
    },
  });

  const micBtn = h("button", {
    class: "btn btn-ghost",
    text: Voice.state === "recording" ? "⏹ Parar" : "🎙 Falar",
    onclick: () => void Voice.toggleRecording(),
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") sendBtn.click();
  });

  const lastReply = [...chatLog].reverse().find((m) => m.role === "assistant");

  main.append(
    h("div", {},
      h("div", { class: "page-title", text: `Olá, ${data.userName}!` }),
      h("div", { class: "page-sub", text: "Escreva, fale ou diga 'Ei, Jarvis'" }),
    ),
    h("div", { class: "card" },
      h("div", { class: "row" }, input, micBtn, sendBtn),
      lastReply
        ? h("div", {
            style: "margin-top:12px;padding-top:12px;border-top:1px solid var(--border);color:#d1d5db",
            text: lastReply.content,
          })
        : h("div", {
            style: "margin-top:10px;font-size:12px;color:var(--muted)",
            text: "Exemplos: 'Jarvis, abre o Spotify' · 'Me lembra de beber água' · 'Guarda na memória que eu gosto de café'",
          }),
    ),
    h("div", { class: "grid-3" },
      quickCard("🎵 Música", "Tocar / Pausar Spotify", () => void Bridge.jarvisMediaControl("play_pause")),
      quickCard("👁 Ler Tela", "Analisar tela atual com Gemini", () => {
        activeTab = "chat";
        void askJarvis("O que está na minha tela agora? Resuma de forma útil.", true);
      }),
      quickCard("⚡ Modo Trabalho", "Abrir VS Code + Spotify", () => {
        void Bridge.jarvisOpenTarget("vscode");
        void Bridge.jarvisOpenTarget("spotify");
      }),
    ),
  );
}

function quickCard(title: string, sub: string, onClick: () => void): HTMLElement {
  const el = h(
    "button",
    { class: "card", style: "text-align:left;cursor:pointer;color:inherit" },
    h("div", { style: "font-weight:600;margin-bottom:4px", text: title }),
    h("div", { style: "font-size:12px;color:var(--muted)", text: sub }),
  );
  el.addEventListener("click", onClick);
  return el;
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
  }
  if (sending) {
    msgs.append(h("div", { class: "msg assistant", text: "Pensando…" }));
  }

  const input = h("input", {
    class: "input",
    style: "flex:1",
    placeholder: "Converse com o Jarvis…",
  }) as HTMLInputElement;

  const sendBtn = h("button", {
    class: "btn",
    text: "Enviar",
    onclick: () => {
      const val = input.value;
      input.value = "";
      void askJarvis(val);
    },
  });
  const screenBtn = h("button", {
    class: "btn btn-ghost",
    title: "Capturar tela e perguntar ao Gemini Vision",
    text: "👁 Ler tela",
    onclick: () => {
      const val = input.value.trim() || "O que está na minha tela agora?";
      input.value = "";
      void askJarvis(val, true);
    },
  });
  const micBtn = h("button", {
    class: "btn btn-ghost",
    text: Voice.state === "recording" ? "⏹" : "🎙",
    onclick: () => void Voice.toggleRecording(),
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") sendBtn.click();
  });

  main.append(
    h("div", { class: "page-title", text: "Conversa" }),
    h("div", { class: "chat-wrap" }, msgs, h("div", { class: "row" }, input, micBtn, screenBtn, sendBtn)),
  );
  requestAnimationFrame(() => {
    msgs.scrollTop = msgs.scrollHeight;
  });
}

function renderMemory(main: HTMLElement) {
  const search = h("input", {
    class: "input",
    style: "flex:1",
    placeholder: "Procurar na memória…",
    value: memoryFilter,
  }) as HTMLInputElement;
  search.addEventListener("input", () => {
    memoryFilter = search.value;
    drawGraph();
  });

  const newMem = h("input", {
    class: "input",
    style: "flex:1",
    placeholder: "Guardar algo novo (ex: Gosto de rock, Trabalho com TypeScript)…",
  }) as HTMLInputElement;

  const addBtn = h("button", {
    class: "btn",
    text: "+ Guardar",
    onclick: async () => {
      const val = newMem.value.trim();
      if (!val) return;
      data.memories.push({
        id: `mem_${Date.now()}`,
        label: val,
        category: "fact",
        createdAt: Math.floor(Date.now() / 1000),
      });
      newMem.value = "";
      await saveData();
      render();
    },
  });

  const canvas = h("canvas", { class: "graph-canvas" }) as HTMLCanvasElement;
  const detailBar = h("div", { class: "row between card", style: "padding:10px 14px" });

  function updateDetail() {
    clear(detailBar);
    if (!selectedMemory) {
      detailBar.append(
        h("span", { style: "color:var(--muted)", text: `${data.memories.length} nós na memória. Arraste ou clique em um nó.` }),
      );
      return;
    }
    detailBar.append(
      h("span", { text: `Selecionado: ${selectedMemory.label} (${selectedMemory.category})` }),
      selectedMemory.category !== "user"
        ? h("button", {
            class: "btn btn-danger",
            text: "Excluir",
            onclick: async () => {
              data.memories = data.memories.filter((m) => m.id !== selectedMemory?.id);
              selectedMemory = null;
              await saveData();
              render();
            },
          })
        : "",
    );
  }

  function drawGraph() {
    cleanupGraph?.();
    cleanupGraph = renderMemoryGraph(canvas, data.memories, data.edges ?? [], memoryFilter, (node) => {
      selectedMemory = node;
      updateDetail();
    });
  }

  main.append(
    h("div", { class: "page-title", text: "Memória" }),
    h("div", { class: "row" }, search),
    canvas,
    detailBar,
    h("div", { class: "row" }, newMem, addBtn),
  );
  updateDetail();
  requestAnimationFrame(drawGraph);
}

function renderTasks(main: HTMLElement) {
  const input = h("input", { class: "input", style: "flex:1", placeholder: "Nova tarefa…" }) as HTMLInputElement;
  const add = async () => {
    const title = input.value.trim();
    if (!title) return;
    data.tasks.push({ id: `t_${Date.now()}`, title, done: false, createdAt: Math.floor(Date.now() / 1000) });
    input.value = "";
    await saveData();
    render();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void add();
  });

  const list = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  for (const t of data.tasks) {
    list.append(
      h("div", { class: "list-item" },
        h("label", { class: "row", style: "cursor:pointer" },
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
        h("button", {
          class: "btn btn-danger",
          text: "Remover",
          onclick: async () => {
            data.tasks = data.tasks.filter((x) => x.id !== t.id);
            await saveData();
            render();
          },
        }),
      ),
    );
  }

  main.append(
    h("div", { class: "page-title", text: "Tarefas" }),
    h("div", { class: "row" }, input, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
    list,
  );
}

function renderReminders(main: HTMLElement) {
  const titleInput = h("input", { class: "input", style: "flex:1", placeholder: "Lembrete…" }) as HTMLInputElement;
  const timeInput = h("input", { class: "input", type: "time", value: "09:00" }) as HTMLInputElement;
  const add = async () => {
    const title = titleInput.value.trim();
    if (!title) return;
    data.reminders.push({ id: `r_${Date.now()}`, title, time: timeInput.value || "09:00", done: false });
    await saveData();
    render();
  };

  const list = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  for (const r of data.reminders) {
    list.append(
      h("div", { class: "list-item" },
        h("span", { text: `⏰ ${r.time} — ${r.title}` }),
        h("button", {
          class: "btn btn-danger",
          text: "Remover",
          onclick: async () => {
            data.reminders = data.reminders.filter((x) => x.id !== r.id);
            await saveData();
            render();
          },
        }),
      ),
    );
  }

  main.append(
    h("div", { class: "page-title", text: "Lembretes" }),
    h("div", { class: "row" }, titleInput, timeInput, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
    list,
  );
}

function renderHabits(main: HTMLElement) {
  const input = h("input", { class: "input", style: "flex:1", placeholder: "Novo hábito…" }) as HTMLInputElement;
  const add = async () => {
    const name = input.value.trim();
    if (!name) return;
    data.habits.push({ id: `h_${Date.now()}`, name, streak: 0, doneToday: false });
    await saveData();
    render();
  };

  const list = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  for (const hb of data.habits) {
    list.append(
      h("div", { class: "list-item" },
        h("div", { class: "row" },
          h("button", {
            class: hb.doneToday ? "btn" : "btn btn-ghost",
            text: hb.doneToday ? "✓ Feito hoje" : "Marcar",
            onclick: async () => {
              hb.doneToday = !hb.doneToday;
              hb.streak += hb.doneToday ? 1 : -1;
              if (hb.streak < 0) hb.streak = 0;
              await saveData();
              render();
            },
          }),
          h("span", { text: `${hb.name} (🔥 ${hb.streak}d)` }),
        ),
        h("button", {
          class: "btn btn-danger",
          text: "Remover",
          onclick: async () => {
            data.habits = data.habits.filter((x) => x.id !== hb.id);
            await saveData();
            render();
          },
        }),
      ),
    );
  }

  main.append(
    h("div", { class: "page-title", text: "Hábitos" }),
    h("div", { class: "row" }, input, h("button", { class: "btn", text: "Adicionar", onclick: () => void add() })),
    list,
  );
}

function renderTimers(main: HTMLElement) {
  const labelIn = h("input", { class: "input", style: "flex:1", placeholder: "Nome do timer (ex: Pomodoro)" }) as HTMLInputElement;
  const minIn = h("input", { class: "input", type: "number", value: "25", style: "width:90px" }) as HTMLInputElement;

  const list = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  for (const tm of timers) {
    const m = Math.floor(tm.remainingSec / 60).toString().padStart(2, "0");
    const s = (tm.remainingSec % 60).toString().padStart(2, "0");
    list.append(
      h("div", { class: "list-item" },
        h("span", { text: `⏱ ${tm.label} — ${m}:${s}` }),
        h("div", { class: "row" },
          h("button", {
            class: "btn btn-ghost",
            text: tm.running ? "Pausar" : "Iniciar",
            onclick: () => {
              tm.running = !tm.running;
              render();
            },
          }),
          h("button", {
            class: "btn btn-danger",
            text: "X",
            onclick: () => {
              const idx = timers.indexOf(tm);
              if (idx >= 0) timers.splice(idx, 1);
              render();
            },
          }),
        ),
      ),
    );
  }

  main.append(
    h("div", { class: "page-title", text: "Timers" }),
    h("div", { class: "row" },
      labelIn,
      minIn,
      h("button", {
        class: "btn",
        text: "Iniciar Timer",
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
    list,
  );
}

function renderMusic(main: HTMLElement) {
  main.append(
    h("div", { class: "page-title", text: "Música & Mídia" }),
    h("div", { class: "card row", style: "flex-wrap:wrap" },
      h("button", { class: "btn btn-ghost", text: "⏮ Anterior", onclick: () => void Bridge.jarvisMediaControl("prev") }),
      h("button", { class: "btn", text: "⏯ Play / Pause", onclick: () => void Bridge.jarvisMediaControl("play_pause") }),
      h("button", { class: "btn btn-ghost", text: "⏭ Próxima", onclick: () => void Bridge.jarvisMediaControl("next") }),
      h("button", { class: "btn btn-ghost", text: "🔉 -", onclick: () => void Bridge.jarvisMediaControl("vol_down") }),
      h("button", { class: "btn btn-ghost", text: "🔊 +", onclick: () => void Bridge.jarvisMediaControl("vol_up") }),
      h("button", { class: "btn btn-ghost", text: "Abrir Spotify", onclick: () => void Bridge.jarvisOpenTarget("spotify") }),
    ),
  );
}

function renderClipboard(main: HTMLElement) {
  const list = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  for (const item of clipboardHistory) {
    list.append(
      h("div", { class: "list-item" },
        h("span", { style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:75%", text: item }),
        h("button", {
          class: "btn btn-ghost",
          text: "Copiar",
          onclick: () => void navigator.clipboard?.writeText(item),
        }),
      ),
    );
  }
  main.append(
    h("div", { class: "page-title", text: "Copiados" }),
    h("div", { class: "page-sub", text: "Histórico recente da sua área de transferência enquanto o Jarvis está aberto." }),
    list,
  );
}

function renderRoutines(main: HTMLElement) {
  const list = h("div", { class: "grid-2" });
  for (const rt of data.routines) {
    list.append(
      h("div", { class: "card" },
        h("div", { style: "font-weight:700;margin-bottom:4px", text: rt.name }),
        h("div", { style: "font-size:12px;color:var(--muted);margin-bottom:10px", text: rt.description }),
        h("button", {
          class: "btn",
          text: "▶ Executar Rotina",
          onclick: async () => {
            for (const target of rt.targets) await Bridge.jarvisOpenTarget(target);
            Sound.play("finish");
          },
        }),
      ),
    );
  }
  main.append(h("div", { class: "page-title", text: "Rotinas" }), list);
}

function renderPC(main: HTMLElement) {
  if (!pcStats) {
    void Bridge.jarvisPcStats().then((st) => {
      if (st) {
        pcStats = st;
        if (activeTab === "pc") render();
      }
    });
  }

  const statCard = (label: string, value: string, sub: string, pct: number) =>
    h(
      "div",
      { class: "card" },
      h("div", { style: "font-size:12px;color:var(--muted)", text: label }),
      h("div", { style: "font-size:22px;font-weight:700;margin:4px 0", text: value }),
      h("div", { style: "font-size:11px;color:var(--muted);margin-bottom:8px", text: sub }),
      h(
        "div",
        { style: "height:6px;background:rgba(255,255,255,0.08);border-radius:99px;overflow:hidden" },
        h("div", {
          style: `height:100%;width:${Math.min(100, pct)}%;background:${pct > 85 ? "#f87171" : "#4ade80"};transition:width .3s`,
        }),
      ),
    );

  const statsGrid = h(
    "div",
    { class: "grid-3" },
    statCard("CPU", pcStats ? `${pcStats.cpuPercent}%` : "…", "Processador", pcStats?.cpuPercent ?? 0),
    statCard(
      "Memória RAM",
      pcStats ? `${pcStats.memPercent}%` : "…",
      pcStats ? `${pcStats.memUsedGb} / ${pcStats.memTotalGb} GB` : "Lendo…",
      pcStats?.memPercent ?? 0,
    ),
    statCard(
      "Disco C:",
      pcStats ? `${pcStats.diskFreeGb} GB livres` : "…",
      pcStats ? `${pcStats.diskPercent}% usado de ${pcStats.diskTotalGb} GB` : "Lendo…",
      pcStats?.diskPercent ?? 0,
    ),
  );

  const scanBtn = h("button", {
    class: "btn",
    text: "🔍 Verificar seu PC",
    onclick: async () => {
      const steps = [
        "Olhando a memória…",
        "Vendo quem usa o processador…",
        "Medindo o disco C:…",
        "Conferindo saúde do sistema…",
      ];
      for (const step of steps) {
        pcScanText = step;
        render();
        await new Promise((r) => setTimeout(r, 450));
      }
      const fresh = await Bridge.jarvisPcStats();
      if (fresh) pcStats = fresh;
      pcScanText = fresh
        ? `Tudo certo! CPU ${fresh.cpuPercent}%, RAM ${fresh.memPercent}% (${fresh.memUsedGb}/${fresh.memTotalGb} GB), ${fresh.diskFreeGb} GB livres no disco.`
        : "Verificação concluída.";
      Sound.play("finish");
      render();
    },
  });

  const apps = [
    ["Spotify", "spotify"],
    ["Google Chrome", "chrome"],
    ["YouTube", "youtube"],
    ["WhatsApp", "whatsapp"],
    ["VS Code", "vscode"],
    ["Terminal", "terminal"],
    ["Calculadora", "calc"],
    ["Explorador", "explorer"],
  ];
  const grid = h("div", { class: "grid-3" });
  for (const [label, target] of apps) {
    grid.append(
      h("button", {
        class: "card",
        style: "cursor:pointer;color:var(--text);font-weight:600;text-align:left",
        text: `🚀 Abrir ${label}`,
        onclick: () => void Bridge.jarvisOpenTarget(target),
      }),
    );
  }
  main.append(
    h("div", { class: "row between" },
      h("div", { class: "page-title", text: "Meu PC" }),
      scanBtn,
    ),
    pcScanText ? h("div", { class: "card", style: "color:#4ade80", text: pcScanText }) : "",
    statsGrid,
    grid,
  );
}

function renderSettings(main: HTMLElement) {
  const mode = settings.jarvisBrainMode ?? "auto";
  const groqOk = brainStatus?.groqConfigured ?? false;
  const gemOk = brainStatus?.geminiConfigured ?? false;

  const brainCard = (id: string, title: string, desc: string) =>
    h(
      "div",
      {
        class: `brain-card ${mode === id ? "selected" : ""}`,
        onclick: async () => {
          settings.jarvisBrainMode = id;
          await saveSettings();
          render();
        },
      },
      h("div", { style: "font-weight:700;margin-bottom:4px", text: title }),
      h("div", { style: "font-size:12px;color:var(--muted)", text: desc }),
    );

  const groqIn = h("input", { class: "input", type: "password", placeholder: groqOk ? "•••••••• (salva)" : "gsk_..." }) as HTMLInputElement;
  const gemIn = h("input", { class: "input", type: "password", placeholder: gemOk ? "•••••••• (salva)" : "AIza..." }) as HTMLInputElement;
  const nameIn = h("input", { class: "input", value: data.userName }) as HTMLInputElement;
  const testOut = h("div", { style: "font-size:12px;color:var(--muted)" });

  main.append(
    h("div", { class: "page-title", text: "Configurações" }),
    h("div", { class: "card" },
      h("div", { style: "font-weight:700;margin-bottom:10px", text: "1. Como o Jarvis pensa (Cérebro)" }),
      h("div", { class: "grid-3" },
        brainCard("auto", "⚡ Automático grátis", "Groq + Gemini — usa o Groq e alterna pro Gemini no limite."),
        brainCard("gemini", "✨ Google Gemini", "Usa o Gemini 2.5 Flash diretamente."),
        brainCard("ollama", "💻 No meu PC", "Ollama local (funciona sem internet)."),
      ),
    ),
    h("div", { class: "card", style: "display:flex;flex-direction:column;gap:10px" },
      h("div", { style: "font-weight:700", text: "2. Conectar as IAs grátis" }),
      h("div", { class: "row between" },
        h("span", { text: "Groq (console.groq.com/keys)" }),
        h("span", { class: `pill ${groqOk ? "ok" : "err"}`, text: groqOk ? "Groq conectado" : "Não conectado" }),
      ),
      h("div", { class: "row" },
        groqIn,
        h("button", {
          class: "btn",
          text: "Salvar Groq",
          onclick: async () => {
            if (!groqIn.value.trim()) return;
            await Bridge.secretSet("groq-api-key", groqIn.value.trim());
            groqIn.value = "";
            brainStatus = await Bridge.jarvisBrainStatus();
            render();
          },
        }),
      ),
      h("div", { class: "row between" },
        h("span", { text: "Gemini (aistudio.google.com/apikey)" }),
        h("span", { class: `pill ${gemOk ? "ok" : "err"}`, text: gemOk ? "Gemini conectado" : "Não conectado" }),
      ),
      h("div", { class: "row" },
        gemIn,
        h("button", {
          class: "btn",
          text: "Salvar Gemini",
          onclick: async () => {
            if (!gemIn.value.trim()) return;
            await Bridge.secretSet("google-api-key", gemIn.value.trim());
            gemIn.value = "";
            brainStatus = await Bridge.jarvisBrainStatus();
            render();
          },
        }),
      ),
      h("div", { class: "row" },
        h("button", {
          class: "btn btn-ghost",
          text: "Testar IA",
          onclick: async () => {
            testOut.textContent = "Testando…";
            try {
              const r = await Bridge.jarvisAsk("Diga oi em 1 frase.", []);
              testOut.textContent = `[${r.providerUsed}] ${r.text}`;
            } catch (e) {
              testOut.textContent = String(e);
            }
          },
        }),
        testOut,
      ),
    ),
    h("div", { class: "card", style: "display:flex;flex-direction:column;gap:10px" },
      h("div", { style: "font-weight:700", text: "3. Personalização & Voz" }),
      h("div", { class: "row" },
        h("span", { text: "Seu nome:" }),
        nameIn,
        h("button", {
          class: "btn btn-ghost",
          text: "Salvar nome",
          onclick: async () => {
            data.userName = nameIn.value.trim() || "Chefe";
            await saveData();
            render();
          },
        }),
      ),
      h("div", { class: "row" },
        h("button", {
          class: "btn btn-ghost",
          text: `Wake Word ("Ei, Jarvis"): ${(settings.jarvisWakeWordEnabled ?? true) ? "Ligada" : "Desligada"}`,
          onclick: async () => {
            settings.jarvisWakeWordEnabled = !(settings.jarvisWakeWordEnabled ?? true);
            await saveSettings();
            Voice.syncWakeWord();
            render();
          },
        }),
        h("button", {
          class: "btn btn-ghost",
          text: `Voz (TTS): ${(settings.jarvisTtsEnabled ?? true) ? "Ligada" : "Desligada"}`,
          onclick: async () => {
            settings.jarvisTtsEnabled = !(settings.jarvisTtsEnabled ?? true);
            await saveSettings();
            render();
          },
        }),
      ),
    ),
  );
}

// ── Shell & Boot ──────────────────────────────────────────────────────────────

function render() {
  cleanupGraph?.();
  cleanupGraph = null;
  clear(root);

  const pendingTasks = data.tasks.filter((t) => !t.done).length;
  const navDefs: { id: TabId; icon: string; label: string; badge?: number }[] = [
    { id: "home", icon: "🏠", label: "Início" },
    { id: "chat", icon: "💬", label: "Conversa" },
    { id: "memory", icon: "🧠", label: "Memória", badge: data.memories.length },
    { id: "tasks", icon: "✅", label: "Tarefas", badge: pendingTasks },
    { id: "reminders", icon: "⏰", label: "Lembretes", badge: data.reminders.length },
    { id: "habits", icon: "🔥", label: "Hábitos" },
    { id: "timers", icon: "⏱", label: "Timers", badge: timers.length },
    { id: "music", icon: "🎵", label: "Música" },
    { id: "clipboard", icon: "📋", label: "Copiados", badge: clipboardHistory.length },
    { id: "routines", icon: "⚡", label: "Rotinas" },
    { id: "pc", icon: "🖥", label: "Meu PC" },
    { id: "settings", icon: "⚙", label: "Configurações" },
  ];

  const navList = h("div", { class: "nav-list" });
  for (const item of navDefs) {
    navList.append(
      h(
        "button",
        {
          class: `nav-item ${activeTab === item.id ? "active" : ""}`,
          onclick: () => {
            activeTab = item.id;
            render();
          },
        },
        h("span", { class: "nav-left" }, h("span", { text: item.icon }), h("span", { text: item.label })),
        item.badge && item.badge > 0 ? h("span", { class: "nav-badge", text: String(item.badge) }) : "",
      ),
    );
  }

  const sidebar = h(
    "aside",
    { class: "sidebar" },
    h(
      "div",
      { class: "brand" },
      h("div", { class: `brand-avatar ${Voice.state === "recording" ? "pulse" : ""}`, text: "🤖" }),
      h(
        "div",
        { class: "brand-info" },
        h("span", { class: "brand-name", text: "Jarvis" }),
        h("span", {
          class: "brand-status",
          text: Voice.state === "recording" ? "Ouvindo…" : sending ? "Pensando…" : "Pronto",
        }),
      ),
    ),
    navList,
  );

  const main = h("main", { class: "main" });
  switch (activeTab) {
    case "home":
      renderHome(main);
      break;
    case "chat":
      renderChat(main);
      break;
    case "memory":
      renderMemory(main);
      break;
    case "tasks":
      renderTasks(main);
      break;
    case "reminders":
      renderReminders(main);
      break;
    case "habits":
      renderHabits(main);
      break;
    case "timers":
      renderTimers(main);
      break;
    case "music":
      renderMusic(main);
      break;
    case "clipboard":
      renderClipboard(main);
      break;
    case "routines":
      renderRoutines(main);
      break;
    case "pc":
      renderPC(main);
      break;
    case "settings":
      renderSettings(main);
      break;
  }

  root.append(sidebar, main);
}

async function main() {
  const boot = await Bridge.boot();
  if (boot) settings = { ...settings, ...boot.settings };
  const loaded = await Bridge.jarvisDataLoad();
  if (loaded) data = loaded;
  brainStatus = await Bridge.jarvisBrainStatus();

  Voice.onCommand = (text) => {
    activeTab = "chat";
    void askJarvis(text);
  };
  Voice.onStateChange = () => render();

  void onEvent<TabId>("dashboard-open-tab", (tab) => {
    activeTab = tab;
    render();
  });

  // Timer tick every second
  window.setInterval(() => {
    let changed = false;
    for (const tm of timers) {
      if (tm.running && tm.remainingSec > 0) {
        tm.remainingSec--;
        changed = true;
        if (tm.remainingSec === 0) {
          tm.running = false;
          Sound.play("finish");
          Voice.speak(`Timer ${tm.label} finalizado!`);
        }
      }
    }
    if (changed && activeTab === "timers") render();
  }, 1000);

  // Clipboard watcher every 2.5s
  window.setInterval(async () => {
    const clip = (await Bridge.jarvisGetClipboard())?.trim() ?? "";
    if (clip && clip !== lastClip) {
      lastClip = clip;
      clipboardHistory.unshift(clip);
      if (clipboardHistory.length > 30) clipboardHistory.pop();
      if (activeTab === "clipboard") render();
    }
  }, 2500);

  render();
}

void main();
