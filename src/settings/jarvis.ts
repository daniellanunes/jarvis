// Jarvis Brain & Voice settings section.

import { Bridge, type JarvisBrainStatus } from "../core/bridge";
import type { Settings } from "../core/state";
import { h, clear } from "../views/dom";

export function jarvisSection(
  settings: Settings,
  status: JarvisBrainStatus | null,
  onSave: () => void,
): HTMLElement {
  let groqSet = status?.groqConfigured ?? false;
  let geminiSet = status?.geminiConfigured ?? false;

  const section = h("section", { class: "card" });
  const header = h("h2", { text: "Jarvis — Cérebro & Voz" });
  const body = h("div", { class: "section-body" });

  function keyRow(
    label: string,
    secretName: string,
    placeholder: string,
    hintUrl: string,
    initialSet: boolean,
    onUpdate: (has: boolean) => void,
  ): HTMLElement {
    let has = initialSet;
    const input = h("input", {
      type: "password",
      placeholder: has ? "•••••••••••• (salva)" : placeholder,
      style: "flex:1;min-width:0",
    }) as HTMLInputElement;

    const badge = h("span", {
      class: has ? "hint" : "hint",
      style: has ? "color:#4ade80" : "color:#f87171",
      text: has ? "Configurada" : "Não configurada",
    });

    const saveBtn = h("button", {
      text: "Salvar",
      onclick: async () => {
        const val = input.value.trim();
        if (!val) return;
        await Bridge.secretSet(secretName, val);
        input.value = "";
        has = true;
        input.placeholder = "•••••••••••• (salva)";
        badge.textContent = "Configurada";
        badge.setAttribute("style", "color:#4ade80");
        onUpdate(true);
      },
    });

    const clearBtn = h("button", {
      text: "Limpar",
      onclick: async () => {
        await Bridge.secretClear(secretName);
        input.value = "";
        has = false;
        input.placeholder = placeholder;
        badge.textContent = "Não configurada";
        badge.setAttribute("style", "color:#f87171");
        onUpdate(false);
      },
    });

    return h(
      "div",
      { style: "display:flex;flex-direction:column;gap:6px;margin-bottom:10px" },
      h(
        "div",
        { class: "row", style: "justify-content:space-between" },
        h("label", { text: label }),
        badge,
      ),
      h("div", { class: "row", style: "gap:6px" }, input, saveBtn, clearBtn),
      h("div", { class: "hint", text: `Obtenha grátis em: ${hintUrl}` }),
    );
  }

  // Brain mode selector
  const modeSelect = h(
    "select",
    {
      onchange: (e: Event) => {
        settings.jarvisBrainMode = (e.target as HTMLSelectElement).value;
        onSave();
      },
    },
    h("option", { value: "auto", text: "Automático (Gemini → Groq → OpenAI → Claude)" }),
    h("option", { value: "groq", text: "Groq (Llama 3.3 — Ultra rápido)" }),
    h("option", { value: "gemini", text: "Google Gemini (3.8 Flash)" }),
    h("option", { value: "ollama", text: "No meu PC (Ollama local)" }),
  ) as HTMLSelectElement;
  modeSelect.value = settings.jarvisBrainMode ?? "auto";

  // Voice toggles
  function makeToggle(initial: boolean, onChange: (v: boolean) => void): HTMLElement {
    const btn = h("button", {
      class: initial ? "switch on" : "switch",
      "aria-pressed": initial,
    });
    btn.addEventListener("click", () => {
      const next = !btn.classList.contains("on");
      btn.classList.toggle("on", next);
      onChange(next);
    });
    return btn;
  }

  const wakeToggle = makeToggle(settings.jarvisWakeWordEnabled ?? true, (v) => {
    settings.jarvisWakeWordEnabled = v;
    onSave();
  });

  const ttsToggle = makeToggle(settings.jarvisTtsEnabled ?? true, (v) => {
    settings.jarvisTtsEnabled = v;
    onSave();
  });

  const confirmToggle = makeToggle(settings.jarvisConfirmSystemActions ?? false, (v) => {
    settings.jarvisConfirmSystemActions = v;
    onSave();
  });

  // Paths & identity (empty = automatic default)
  type TextField = "jarvisUserName" | "jarvisObsidianVault" | "jarvisProjectsDir" | "jarvisOpenbotWebhookUrl";
  function textRow(label: string, field: TextField, placeholder: string, hint: string): HTMLElement {
    const input = h("input", {
      type: "text",
      placeholder,
      value: settings[field] ?? "",
      style: "flex:1;min-width:0",
    }) as HTMLInputElement;
    input.addEventListener("change", () => {
      settings[field] = input.value.trim();
      onSave();
    });
    return h(
      "div",
      { style: "display:flex;flex-direction:column;gap:6px;margin-bottom:10px" },
      h("label", { text: label }),
      input,
      h("div", { class: "hint", text: hint }),
    );
  }

  // Test Brain button
  const testResult = h("div", { class: "hint", text: "" });
  const testBtn = h("button", {
    text: "Testar Cérebro",
    onclick: async () => {
      testResult.textContent = "Consultando Jarvis…";
      testResult.setAttribute("style", "color:#9ca3af");
      try {
        const reply = await Bridge.jarvisAsk("Diga olá em uma frase curta.", []);
        testResult.textContent = `[${reply.providerUsed}] ${reply.text}`;
        testResult.setAttribute("style", "color:#4ade80");
      } catch (err) {
        testResult.textContent = String(err).replace(/^Error:\s*/, "");
        testResult.setAttribute("style", "color:#f87171");
      }
    },
  });

  clear(body);
  body.append(
    h(
      "div",
      { class: "row", style: "margin-bottom:10px" },
      h("label", { text: "Modo do Cérebro" }),
      modeSelect,
    ),
    keyRow(
      "Chave Groq (IA + Voz Whisper)",
      "groq-api-key",
      "gsk_...",
      "https://console.groq.com/keys",
      groqSet,
      (v) => {
        groqSet = v;
      },
    ),
    keyRow(
      "Chave Google Gemini (Fallback)",
      "google-api-key",
      "AIza...",
      "https://aistudio.google.com/apikey",
      geminiSet,
      (v) => {
        geminiSet = v;
      },
    ),
    h(
      "div",
      { class: "row", style: "margin-top:6px" },
      h("label", { text: 'Ativação contínua ("Ei, Jarvis")' }),
      wakeToggle,
    ),
    h(
      "div",
      { class: "row" },
      h("label", { text: "Responder falando (TTS pt-BR)" }),
      ttsToggle,
    ),
    h(
      "div",
      { class: "row" },
      h("label", { text: "Pedir confirmação antes de comandos PowerShell/CMD" }),
      confirmToggle,
    ),
    textRow("Seu nome", "jarvisUserName", "Automático (nome da conta do Windows)", "Como o Jarvis te chama."),
    textRow(
      "Pasta da Memória Unificada (Obsidian)",
      "jarvisObsidianVault",
      "Automático: Documentos\\Obsidian Vault\\Memoria-Unificada",
      "Pasta com INDEX.md e projetos/. Vazio = padrão.",
    ),
    textRow(
      "Pasta de projetos",
      "jarvisProjectsDir",
      "Automático: D:\\Projetos ou ~\\Projetos",
      "Onde o Jarvis procura seus projetos. Vazio = padrão.",
    ),
    textRow(
      "Webhook do OpenBot",
      "jarvisOpenbotWebhookUrl",
      "Automático (webhook padrão)",
      "URL https:// que recebe os comandos delegados.",
    ),
    h("div", { class: "row", style: "margin-top:8px;gap:10px" }, testBtn, testResult),
  );

  section.append(header, body);
  return section;
}
