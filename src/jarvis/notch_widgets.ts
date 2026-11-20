// Notch pill widgets: weather, PC usage rings (CPU / RAM / GPU / disk) and the
// frame rate of the game in the foreground. One long-lived element, patched
// in place every second — re-creating it would drop the hover tooltip.

import { Bridge, type PcMetrics, type WeatherNow } from "../core/bridge";
import type { Settings } from "../core/state";

const SVG_NS = "http://www.w3.org/2000/svg";

/** 24×24 line icons, drawn like the rest of the notch. */
const WEATHER_ICONS: Record<string, string> = {
  sun: "M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10z M12 1v2 M12 21v2 M4.2 4.2l1.4 1.4 M18.4 18.4l1.4 1.4 M1 12h2 M21 12h2 M4.2 19.8l1.4-1.4 M18.4 5.6l1.4-1.4",
  moon: "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z",
  partly: "M8 3v1.5 M3.6 5.6l1 1 M2 10h1.5 M12.4 5.6l-1 1 M5.3 12.6A4 4 0 1 1 11.9 8 M17.5 20H9a4 4 0 1 1 .9-7.9A5 5 0 0 1 19 13.5a3.3 3.3 0 0 1-1.5 6.5z",
  cloudy: "M17.5 19H8a5 5 0 1 1 1.2-9.9A6 6 0 0 1 20.6 12a3.6 3.6 0 0 1-3.1 7z",
  fog: "M4 14h16 M6 18h12 M17.5 10H8a4 4 0 1 1 1-7.9A5 5 0 0 1 18.5 5",
  drizzle: "M8 19v1 M12 21v1 M16 19v1 M17.5 15H8a5 5 0 1 1 1.2-9.9A6 6 0 0 1 20.6 8a3.6 3.6 0 0 1-3.1 7z",
  rain: "M8 18l-1 3 M12 18l-1 3 M16 18l-1 3 M17.5 14H8a5 5 0 1 1 1.2-9.9A6 6 0 0 1 20.6 7a3.6 3.6 0 0 1-3.1 7z",
  snow: "M8 18h.01 M12 20h.01 M16 18h.01 M10 22h.01 M14 22h.01 M17.5 14H8a5 5 0 1 1 1.2-9.9A6 6 0 0 1 20.6 7a3.6 3.6 0 0 1-3.1 7z",
  storm: "M13 12l-3 5h4l-3 5 M17.5 15h-1 M7 15a5 5 0 1 1 2.2-9.9A6 6 0 0 1 20.6 8a3.6 3.6 0 0 1-2.1 6.6",
};

function weatherIcon(w: WeatherNow): string {
  if (w.kind === "clear") return w.isDay ? WEATHER_ICONS.sun : WEATHER_ICONS.moon;
  return WEATHER_ICONS[w.kind] ?? WEATHER_ICONS.cloudy;
}

const fmt = (n: number, digits = 0) => n.toLocaleString("pt-BR", { maximumFractionDigits: digits, minimumFractionDigits: digits });

type RingKey = "cpu" | "ram" | "gpu" | "disk";
const RING_LABEL: Record<RingKey, string> = { cpu: "CPU", ram: "RAM", gpu: "GPU", disk: "SSD" };
const R = 8.5;
const CIRC = 2 * Math.PI * R;

function level(pct: number): string {
  return pct >= 90 ? "hot" : pct >= 75 ? "warm" : "ok";
}

export interface NotchWidgets {
  /** The element to put in the pill (the same one every render). */
  el: HTMLElement;
  /** True when something is shown, so the pill knows to make room. */
  visible(): boolean;
  /** Settings changed: start/stop what is shown. */
  configure(s: Settings): void;
  /** null until a reading arrives; false when FPS cannot be measured (no admin). */
  fpsAvailable(): boolean | null;
  /** Weather for a city typed in Ajustes (does not touch the shown one). */
  testCity(city: string): Promise<WeatherNow>;
  dispose(): void;
}

export function createNotchWidgets(onLayout: () => void): NotchWidgets {
  const el = document.createElement("div");
  el.className = "notch-widgets";
  el.addEventListener("click", (e) => e.stopPropagation()); // hovering/clicking a ring does not open the panel

  const tip = document.createElement("div");
  tip.className = "notch-tip";
  let tipFor: (() => string) | null = null;
  let tipAnchor: HTMLElement | null = null;

  function placeTip() {
    if (!tipFor || !tipAnchor) return;
    tip.textContent = tipFor();
    const a = tipAnchor.getBoundingClientRect();
    const w = tip.offsetWidth;
    const left = Math.min(Math.max(8, a.left + a.width / 2 - w / 2), window.innerWidth - w - 8);
    tip.style.left = `${left}px`;
    tip.style.top = `${a.bottom + 8}px`;
  }
  function hoverTip(anchor: HTMLElement, text: () => string) {
    anchor.addEventListener("mouseenter", () => {
      tipFor = text;
      tipAnchor = anchor;
      if (!tip.isConnected) document.body.append(tip);
      tip.classList.add("show");
      placeTip();
    });
    anchor.addEventListener("mouseleave", () => {
      if (tipAnchor !== anchor) return;
      tipFor = null;
      tipAnchor = null;
      tip.classList.remove("show");
    });
  }

  // ── FPS ───────────────────────────────────────────────────────────────────
  const fpsEl = document.createElement("div");
  fpsEl.className = "nw-fps";
  const fpsNum = document.createElement("b");
  const fpsUnit = document.createElement("span");
  fpsUnit.textContent = "FPS";
  fpsEl.append(fpsNum, fpsUnit);
  hoverTip(fpsEl, () => (metrics?.fpsApp ? `${metrics.fpsApp} — ${metrics.fps ?? 0} quadros por segundo` : "FPS"));

  // ── Weather ───────────────────────────────────────────────────────────────
  const weatherEl = document.createElement("div");
  weatherEl.className = "nw-weather";
  const weatherIco = document.createElement("span");
  weatherIco.className = "nw-weather-ico";
  const weatherTemp = document.createElement("b");
  const weatherRain = document.createElement("span");
  weatherRain.className = "nw-rain";
  weatherEl.append(weatherIco, weatherTemp, weatherRain);
  hoverTip(weatherEl, () =>
    weather
      ? `${weather.place} · ${weather.label}\nAgora ${fmt(weather.temp)}° (sensação ${fmt(weather.feels)}°)\nMáx ${fmt(weather.max)}° · Mín ${fmt(weather.min)}° · Chuva ${weather.rainChance}%`
      : weatherErr || "Clima",
  );

  // ── Rings ─────────────────────────────────────────────────────────────────
  const ringsEl = document.createElement("div");
  ringsEl.className = "nw-rings";
  const rings = {} as Record<RingKey, { root: HTMLElement; arc: SVGCircleElement; num: HTMLElement }>;
  for (const k of ["cpu", "ram", "gpu", "disk"] as RingKey[]) {
    const root = document.createElement("div");
    root.className = "nw-ring";
    root.dataset.k = k;
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("viewBox", "0 0 22 22");
    svg.setAttribute("width", "22");
    svg.setAttribute("height", "22");
    const track = document.createElementNS(SVG_NS, "circle");
    const arc = document.createElementNS(SVG_NS, "circle");
    for (const c of [track, arc]) {
      c.setAttribute("cx", "11");
      c.setAttribute("cy", "11");
      c.setAttribute("r", String(R));
      c.setAttribute("fill", "none");
      c.setAttribute("stroke-width", "2.4");
    }
    track.setAttribute("class", "nw-track");
    arc.setAttribute("class", "nw-arc");
    arc.setAttribute("stroke-dasharray", String(CIRC));
    arc.setAttribute("stroke-dashoffset", String(CIRC));
    arc.setAttribute("transform", "rotate(-90 11 11)");
    arc.setAttribute("stroke-linecap", "round");
    svg.append(track, arc);
    const num = document.createElement("span");
    num.className = "nw-ring-num";
    root.append(svg, num);
    ringsEl.append(root);
    rings[k] = { root, arc, num };
    hoverTip(root, () => ringTip(k));
  }

  // ── Internet ──────────────────────────────────────────────────────────────
  const netEl = document.createElement("div");
  netEl.className = "nw-net";
  const netDot = document.createElement("span");
  netDot.className = "nw-net-dot";
  const netText = document.createElement("span");
  netEl.append(netDot, netText);
  hoverTip(netEl, () =>
    !metrics || metrics.pingMs == null && metrics.online
      ? "Internet — medindo…"
      : metrics.online
        ? `Internet — ping ${metrics.pingMs} ms\nCloudflare 1.1.1.1`
        : "Sem internet\nNão consegui alcançar a Cloudflare nem o Google.",
  );

  el.append(fpsEl, weatherEl, netEl, ringsEl);

  let settings: Settings | null = null;
  let metrics: PcMetrics | null = null;
  let weather: WeatherNow | null = null;
  let weatherErr = "";
  let weatherTimer: number | null = null;
  let weatherCity = "";

  /** Temperatures need LibreHardwareMonitor running (Windows has no API for them). */
  function tempHint(m: PcMetrics): string {
    return m.sensors ? "" : "\nTemperatura: abra o LibreHardwareMonitor para aparecer aqui.";
  }

  function ringTip(k: RingKey): string {
    const m = metrics;
    if (!m) return RING_LABEL[k];
    switch (k) {
      case "cpu":
        return `Processador — ${fmt(m.cpu)}%${m.cpuTemp != null ? ` · ${fmt(m.cpuTemp)} °C` : ""}\n${m.cpuName}${tempHint(m)}`;
      case "ram":
        return `Memória RAM — ${fmt(m.ram)}%\n${fmt(m.ramUsedGb, 1)} GB de ${fmt(m.ramTotalGb, 1)} GB em uso`;
      case "gpu": {
        const t = m.gpuTemp != null ? ` · ${fmt(m.gpuTemp)} °C` : "";
        return m.gpu == null ? `Placa de vídeo${t}\n${m.gpuName}${tempHint(m)}` : `Placa de vídeo — ${fmt(m.gpu)}%${t}\n${m.gpuName}${tempHint(m)}`;
      }
      case "disk":
        return `Disco C: — ${fmt(m.disk)}% ocupado\n${fmt(m.diskFreeGb)} GB livres de ${fmt(m.diskTotalGb)} GB`;
    }
  }

  function setRing(k: RingKey, pct: number | null) {
    const r = rings[k];
    r.root.hidden = pct == null;
    if (pct == null) return;
    const p = Math.max(0, Math.min(100, pct));
    r.arc.setAttribute("stroke-dashoffset", String(CIRC * (1 - p / 100)));
    r.root.dataset.level = level(p);
    r.num.textContent = String(Math.round(p));
  }

  let lastShape = "";
  function paint() {
    const s = settings;
    const showPc = Boolean(s?.jarvisNotchPc && metrics);
    const showFps = Boolean(s?.jarvisNotchFps && metrics?.fps != null);
    const showWeather = Boolean(s?.jarvisNotchWeather && weather);
    const showNet = Boolean(s?.jarvisNotchNet && metrics?.netOn && (metrics.pingMs != null || !metrics.online));
    netEl.hidden = !showNet;
    if (showNet && metrics) {
      const off = !metrics.online;
      netEl.dataset.state = off ? "off" : metrics.pingMs! > 120 ? "slow" : "ok";
      netText.textContent = off ? "Sem internet" : `${metrics.pingMs} ms`;
    }

    ringsEl.hidden = !showPc;
    if (showPc && metrics) {
      setRing("cpu", metrics.cpu);
      setRing("ram", metrics.ram);
      setRing("gpu", metrics.gpu);
      setRing("disk", metrics.disk);
    }
    fpsEl.hidden = !showFps;
    if (showFps && metrics) {
      fpsNum.textContent = String(metrics.fps);
      fpsEl.dataset.level = metrics.fps! >= 55 ? "ok" : metrics.fps! >= 30 ? "warm" : "hot";
    }
    weatherEl.hidden = !showWeather;
    if (showWeather && weather) {
      weatherIco.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="${weatherIcon(weather)}"/></svg>`;
      weatherEl.dataset.kind = weather.kind;
      weatherTemp.textContent = `${Math.round(weather.temp)}°`;
      weatherRain.textContent = weather.rainChance >= 30 ? `${weather.rainChance}%` : "";
      weatherRain.hidden = weather.rainChance < 30;
    }
    el.hidden = !(showPc || showFps || showWeather || showNet);
    placeTip();

    // Only a change of what is shown moves the pill's edges.
    const shape = `${showPc}${showFps}${showWeather}${showNet}${netEl.dataset.state}${fpsNum.textContent?.length}${weatherRain.hidden}${metrics?.gpu == null}`;
    if (shape !== lastShape) {
      lastShape = shape;
      onLayout();
    }
  }

  async function loadWeather() {
    if (!settings?.jarvisNotchWeather || !weatherCity) return;
    try {
      weather = await Bridge.weatherNow();
      weatherErr = "";
    } catch (e) {
      weatherErr = String(e);
    }
    paint();
  }

  const unlisten = Bridge.onPcMetrics((m) => {
    metrics = m;
    paint();
  });

  return {
    el,
    visible: () => !el.hidden,
    fpsAvailable: () => (metrics ? metrics.fpsAvailable : null),
    configure(s) {
      settings = s;
      const city = (s.jarvisWeatherCity ?? "").trim();
      if (!s.jarvisNotchWeather || !city) {
        weather = null;
        if (weatherTimer != null) window.clearInterval(weatherTimer);
        weatherTimer = null;
      } else if (city !== weatherCity || weatherTimer == null) {
        if (city !== weatherCity) weather = null;
        weatherCity = city;
        if (weatherTimer != null) window.clearInterval(weatherTimer);
        weatherTimer = window.setInterval(() => void loadWeather(), 10 * 60 * 1000);
        void loadWeather();
      }
      if (!s.jarvisNotchPc && !s.jarvisNotchFps && !s.jarvisNotchNet) metrics = null;
      paint();
    },
    testCity: (city) => Bridge.weatherNow(city),
    dispose() {
      if (weatherTimer != null) window.clearInterval(weatherTimer);
      void unlisten.then((f) => f());
      tip.remove();
    },
  };
}
