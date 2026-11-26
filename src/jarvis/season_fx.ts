// The living part of the seasonal looks (seasons.ts): a canvas over the open
// panel with each season's particles — snow, fireworks, confetti, bats,
// balloons, embers — plus the garland under the pill, the corner ornament
// and the season card on Início. One canvas for the whole app, moved into
// each new panel so the animation never restarts; 30 fps at most, and it
// sleeps while the panel is closed or the window hidden.

import { h } from "../views/dom";
import type { SeasonId } from "./seasons";

// ── Particles ───────────────────────────────────────────────────────────────

interface P {
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  a: number; // opacity
  rot: number;
  vr: number;
  life: number; // seconds left (fireworks), Infinity otherwise
  color: string;
  kind: number;
  phase: number;
}

const rand = (a: number, b: number) => a + Math.random() * (b - a);
const pick = <T,>(list: readonly T[]) => list[Math.floor(Math.random() * list.length)];

const PALETTE: Record<SeasonId, string[]> = {
  natal: ["#ffffff"],
  anonovo: ["#facc15", "#fde68a", "#ffffff", "#f472b6", "#60a5fa"],
  carnaval: ["#ec4899", "#a855f7", "#facc15", "#22c55e", "#38bdf8", "#f97316"],
  pascoa: ["#f9a8d4", "#a7f3d0", "#fde68a", "#c4b5fd", "#bae6fd"],
  junina: ["#fb923c", "#f97316", "#facc15", "#fde047"],
  halloween: ["#f97316", "#a855f7", "#fb923c"],
  aniversario: ["#f472b6", "#60a5fa", "#facc15", "#34d399", "#a78bfa", "#fb7185"],
};

class Fx {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  season: SeasonId | null = null;
  parts: P[] = [];
  w = 0;
  h = 0;
  dpr = 1;
  last = 0;
  raf = 0;
  nextBurst = 0;
  nextSpawn = 0;
  observer: ResizeObserver;

  constructor() {
    this.canvas = h("canvas", { class: "season-canvas" }) as HTMLCanvasElement;
    this.ctx = this.canvas.getContext("2d");
    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(this.canvas);
    document.addEventListener("visibilitychange", () => this.wake());
  }

  resize() {
    const r = this.canvas.getBoundingClientRect();
    this.dpr = Math.min(2, window.devicePixelRatio || 1);
    this.w = r.width;
    this.h = r.height;
    this.canvas.width = Math.max(1, Math.round(r.width * this.dpr));
    this.canvas.height = Math.max(1, Math.round(r.height * this.dpr));
  }

  setSeason(s: SeasonId | null) {
    if (s === this.season) return;
    this.season = s;
    this.parts = [];
    this.nextBurst = 0;
    this.wake();
  }

  wake() {
    if (!this.raf && this.season && this.canvas.isConnected && !document.hidden) {
      this.last = performance.now();
      this.raf = requestAnimationFrame((t) => this.frame(t));
    }
  }

  /** How many particles of the season float at once. */
  target(): number {
    const area = (this.w * this.h) / 100_000;
    switch (this.season) {
      case "natal": return Math.round(30 * area);
      case "carnaval": return Math.round(26 * area);
      case "aniversario": return Math.round(18 * area);
      case "pascoa": return Math.round(18 * area);
      case "junina": return Math.round(30 * area);
      case "halloween": return Math.round(16 * area);
      case "anonovo": return Math.round(22 * area);
      default: return 0;
    }
  }

  spawn(initial: boolean): P {
    const s = this.season!;
    const W = this.w;
    const H = this.h;
    const color = pick(PALETTE[s]);
    const base: P = { x: rand(0, W), y: initial ? rand(0, H) : -10, vx: 0, vy: 0, r: 2, a: 1, rot: rand(0, 6.28), vr: 0, life: Infinity, color, kind: 0, phase: rand(0, 6.28) };
    switch (s) {
      case "natal": {
        // Near flakes are bigger, faster and brighter: a little depth.
        const z = Math.random();
        return { ...base, r: 0.8 + z * 2.4, vy: 14 + z * 30, vx: rand(-6, 6), a: 0.3 + z * 0.5, kind: z > 0.9 ? 1 : 0, vr: rand(-0.6, 0.6) };
      }
      case "carnaval":
        return { ...base, r: rand(2.5, 4.5), vy: rand(25, 55), vx: rand(-15, 15), vr: rand(-4, 4), a: 0.9, kind: Math.random() < 0.12 ? 1 : 0 };
      case "aniversario":
        // A few balloons at a time; the rest is confetti.
        if (this.count(1) < 5 && Math.random() < 0.3) return { ...base, y: initial ? rand(H * 0.4, H) : H + 30, r: rand(9, 13), vy: -rand(16, 28), vx: rand(-4, 4), a: 0.8, kind: 1 };
        return { ...base, r: rand(2, 4), vy: rand(22, 45), vx: rand(-12, 12), vr: rand(-4, 4), a: 0.9 };
      case "pascoa":
        return { ...base, r: rand(3, 5.5), vy: rand(10, 20), vx: rand(-8, 8), vr: rand(-1, 1), a: 0.55, kind: Math.random() < 0.4 ? 1 : 0 };
      case "junina":
        // Embers rising from the bottom.
        return { ...base, y: initial ? rand(H * 0.3, H) : H + 5, r: rand(0.8, 2.2), vy: -rand(18, 45), vx: rand(-8, 8), a: rand(0.5, 1), life: rand(2.5, 6) };
      case "halloween":
        if (this.count(1) < 4 && Math.random() < 0.4) return { ...base, x: initial ? rand(0, W) : -20, y: rand(10, H * 0.55), r: rand(7, 12), vx: rand(25, 55), vy: rand(-4, 4), a: 0.8, kind: 1, color: "#1c1326" };
        return { ...base, y: initial ? rand(0, H) : H + 5, r: rand(0.8, 2), vy: -rand(8, 20), vx: rand(-5, 5), a: rand(0.4, 0.9), life: rand(3, 7) };
      case "anonovo":
        return { ...base, r: rand(0.8, 1.8), vy: rand(6, 16), vx: rand(-4, 4), a: rand(0.4, 0.9), kind: 2 };
    }
    return base;
  }

  count(kind: number): number {
    let n = 0;
    for (const p of this.parts) if (p.kind === kind) n++;
    return n;
  }

  /** New Year: a rocket bursting into a ring of sparks. */
  burst() {
    const cx = rand(this.w * 0.15, this.w * 0.85);
    const cy = rand(this.h * 0.12, this.h * 0.45);
    const color = pick(PALETTE.anonovo);
    const n = 46;
    const speed = rand(70, 120);
    for (let i = 0; i < n; i++) {
      const ang = (i / n) * Math.PI * 2 + rand(-0.05, 0.05);
      const v = speed * rand(0.75, 1.05);
      this.parts.push({ x: cx, y: cy, vx: Math.cos(ang) * v, vy: Math.sin(ang) * v, r: rand(1.1, 1.9), a: 1, rot: 0, vr: 0, life: rand(1.1, 1.7), color, kind: 3, phase: 0 });
    }
  }

  frame(t: number) {
    this.raf = 0;
    if (!this.season || !this.canvas.isConnected || document.hidden || !this.ctx) return;
    const dt = Math.min(0.05, (t - this.last) / 1000);
    // 30 fps is plenty for drifting things.
    if (t - this.last < 31) {
      this.raf = requestAnimationFrame((tt) => this.frame(tt));
      return;
    }
    this.last = t;
    if (!this.w) this.resize();
    const ctx = this.ctx;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.clearRect(0, 0, this.w, this.h);

    // Keep the season's crowd.
    const want = this.target();
    const floating = this.parts.filter((p) => p.kind !== 3).length;
    if (floating < want) {
      const initial = this.parts.length === 0;
      for (let i = 0; i < (initial ? want : Math.min(2, want - floating)); i++) this.parts.push(this.spawn(initial));
    }
    if (this.season === "anonovo" && t > this.nextBurst) {
      this.burst();
      this.nextBurst = t + rand(1100, 2200);
    }

    const keep: P[] = [];
    for (const p of this.parts) {
      p.phase += dt;
      p.rot += p.vr * dt;
      if (p.kind === 3) {
        p.vy += 60 * dt; // gravity
        p.vx *= 0.985;
        p.vy *= 0.985;
      }
      const sway = this.season === "natal" || this.season === "pascoa" ? Math.sin(p.phase * 1.3) * 10 : 0;
      p.x += (p.vx + sway) * dt;
      p.y += p.vy * dt;
      if (Number.isFinite(p.life)) {
        p.life -= dt;
        if (p.life <= 0) continue;
      }
      if (p.y > this.h + 30 || p.y < -40 || p.x < -40 || p.x > this.w + 40) continue;
      this.draw(ctx, p);
      keep.push(p);
    }
    this.parts = keep;
    this.raf = requestAnimationFrame((tt) => this.frame(tt));
  }

  draw(ctx: CanvasRenderingContext2D, p: P) {
    const fade = Number.isFinite(p.life) ? Math.min(1, p.life / 0.6) : 1;
    ctx.globalAlpha = p.a * fade;
    ctx.fillStyle = p.color;
    ctx.strokeStyle = p.color;
    switch (this.season) {
      case "natal":
        if (p.kind === 1) {
          // A six-armed flake.
          ctx.save();
          ctx.translate(p.x, p.y);
          ctx.rotate(p.rot);
          ctx.lineWidth = 0.9;
          ctx.beginPath();
          for (let i = 0; i < 6; i++) {
            ctx.moveTo(0, 0);
            ctx.lineTo(0, -p.r * 2.2);
            ctx.rotate(Math.PI / 3);
          }
          ctx.stroke();
          ctx.restore();
        } else {
          ctx.beginPath();
          ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
          ctx.fill();
        }
        break;
      case "carnaval":
      case "aniversario":
        if (p.kind === 1 && this.season === "aniversario") {
          // A balloon on a string.
          ctx.beginPath();
          ctx.ellipse(p.x, p.y, p.r * 0.82, p.r, 0, 0, Math.PI * 2);
          ctx.fill();
          ctx.globalAlpha *= 0.35;
          ctx.fillStyle = "#fff";
          ctx.beginPath();
          ctx.ellipse(p.x - p.r * 0.3, p.y - p.r * 0.35, p.r * 0.18, p.r * 0.28, -0.5, 0, Math.PI * 2);
          ctx.fill();
          ctx.globalAlpha = p.a * 0.6;
          ctx.strokeStyle = "rgba(255,255,255,0.5)";
          ctx.lineWidth = 0.7;
          ctx.beginPath();
          ctx.moveTo(p.x, p.y + p.r);
          ctx.quadraticCurveTo(p.x + Math.sin(p.phase * 2) * 4, p.y + p.r + 10, p.x, p.y + p.r + 20);
          ctx.stroke();
        } else if (p.kind === 1) {
          // A streamer.
          ctx.lineWidth = 1.6;
          ctx.beginPath();
          for (let i = 0; i <= 10; i++) {
            const yy = p.y + i * 3.2;
            const xx = p.x + Math.sin(p.phase * 3 + i * 0.9) * 4;
            if (i === 0) ctx.moveTo(xx, yy);
            else ctx.lineTo(xx, yy);
          }
          ctx.stroke();
        } else {
          // Confetti: a little card turning over.
          ctx.save();
          ctx.translate(p.x, p.y);
          ctx.rotate(p.rot);
          ctx.scale(1, Math.cos(p.phase * 5));
          ctx.fillRect(-p.r, -p.r * 0.6, p.r * 2, p.r * 1.2);
          ctx.restore();
        }
        break;
      case "pascoa":
        if (p.kind === 1) {
          // A little egg.
          ctx.save();
          ctx.translate(p.x, p.y);
          ctx.rotate(Math.sin(p.phase) * 0.4);
          ctx.beginPath();
          ctx.ellipse(0, 0, p.r * 0.8, p.r * 1.05, 0, 0, Math.PI * 2);
          ctx.fill();
          ctx.restore();
        } else {
          // A petal.
          ctx.save();
          ctx.translate(p.x, p.y);
          ctx.rotate(p.rot);
          ctx.beginPath();
          ctx.ellipse(0, 0, p.r, p.r * 0.45, 0, 0, Math.PI * 2);
          ctx.fill();
          ctx.restore();
        }
        break;
      case "halloween":
        if (p.kind === 1) {
          // A bat, flapping.
          const flap = Math.sin(p.phase * 14);
          ctx.save();
          ctx.translate(p.x, p.y + Math.sin(p.phase * 3) * 3);
          ctx.fillStyle = "#0b0712";
          ctx.globalAlpha = 0.85;
          ctx.beginPath();
          ctx.moveTo(0, 0);
          ctx.quadraticCurveTo(-p.r * 0.6, -p.r * (0.2 + 0.6 * flap), -p.r * 1.3, -p.r * 0.1 * flap);
          ctx.quadraticCurveTo(-p.r * 0.7, p.r * 0.1, -p.r * 0.35, p.r * 0.35);
          ctx.quadraticCurveTo(0, p.r * 0.15, p.r * 0.35, p.r * 0.35);
          ctx.quadraticCurveTo(p.r * 0.7, p.r * 0.1, p.r * 1.3, -p.r * 0.1 * flap);
          ctx.quadraticCurveTo(p.r * 0.6, -p.r * (0.2 + 0.6 * flap), 0, 0);
          ctx.fill();
          ctx.restore();
        } else {
          ctx.shadowColor = p.color;
          ctx.shadowBlur = 6;
          ctx.beginPath();
          ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
          ctx.fill();
          ctx.shadowBlur = 0;
        }
        break;
      case "junina":
        ctx.shadowColor = p.color;
        ctx.shadowBlur = 8;
        ctx.beginPath();
        ctx.arc(p.x + Math.sin(p.phase * 4) * 2, p.y, p.r, 0, Math.PI * 2);
        ctx.fill();
        ctx.shadowBlur = 0;
        break;
      case "anonovo":
        if (p.kind === 3) {
          ctx.shadowColor = p.color;
          ctx.shadowBlur = 6;
          ctx.beginPath();
          ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
          ctx.fill();
          ctx.shadowBlur = 0;
        } else {
          // A gold twinkle.
          const s = p.r * (1.6 + Math.sin(p.phase * 4) * 0.8);
          ctx.globalAlpha = p.a * (0.5 + 0.5 * Math.sin(p.phase * 3));
          ctx.beginPath();
          ctx.moveTo(p.x, p.y - s * 2);
          ctx.lineTo(p.x + s * 0.4, p.y);
          ctx.lineTo(p.x, p.y + s * 2);
          ctx.lineTo(p.x - s * 0.4, p.y);
          ctx.closePath();
          ctx.fill();
        }
        break;
    }
    ctx.globalAlpha = 1;
  }
}

let fx: Fx | null = null;

/** The season's canvas, inside `panel` (moved there, never rebuilt). */
export function attachSeasonCanvas(panel: HTMLElement, season: SeasonId | null) {
  if (!season) {
    fx?.setSeason(null);
    fx?.canvas.remove();
    return;
  }
  if (typeof ResizeObserver === "undefined" || typeof requestAnimationFrame === "undefined") return;
  fx ??= new Fx();
  if (fx.canvas.parentElement !== panel) panel.append(fx.canvas);
  fx.setSeason(season);
  fx.wake();
}

// ── The garland under the pill ──────────────────────────────────────────────

const BULBS: Record<SeasonId, string[]> = {
  natal: ["#ef4444", "#22c55e", "#facc15", "#3b82f6", "#f97316"],
  anonovo: ["#facc15", "#fde68a", "#fef3c7"],
  carnaval: ["#ec4899", "#a855f7", "#facc15", "#22c55e", "#38bdf8"],
  pascoa: ["#f9a8d4", "#a7f3d0", "#fde68a", "#c4b5fd"],
  junina: ["#ef4444", "#facc15", "#22c55e", "#3b82f6", "#f97316"],
  halloween: ["#f97316", "#a855f7"],
  aniversario: ["#f472b6", "#60a5fa", "#facc15", "#34d399", "#a78bfa"],
};

/** A wire hanging in loops under the pill, with bulbs (or little flags) on it. */
export function garland(season: SeasonId): HTMLElement {
  const flags = season === "junina" || season === "aniversario";
  const n = 13;
  const colors = BULBS[season];
  const loops = 4;
  // The wire dips between the hooks: y = sag · |sin(π·loops·x)|.
  const wire = Array.from({ length: 81 }, (_, i) => {
    const x = i / 80;
    return `${(x * 100).toFixed(2)},${(1.5 + 6.5 * Math.abs(Math.sin(Math.PI * loops * x))).toFixed(2)}`;
  }).join(" ");
  const el = h("div", { class: `season-garland ${flags ? "flags" : "bulbs"}` });
  el.innerHTML = `<svg viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"><polyline points="${wire}" fill="none" stroke="rgba(30,30,30,0.9)" stroke-width="0.7" vector-effect="non-scaling-stroke"/></svg>`;
  for (let i = 0; i < n; i++) {
    const x = (i + 0.5) / n;
    const y = 1.5 + 6.5 * Math.abs(Math.sin(Math.PI * loops * x));
    const b = h("i", { class: "season-bulb" });
    b.style.left = `${x * 100}%`;
    b.style.top = `${y * 10}%`;
    b.style.setProperty("--c", colors[i % colors.length]);
    b.style.animationDelay = `${(i % 5) * 0.32}s`;
    el.append(b);
  }
  return el;
}

// ── The corner ornament ─────────────────────────────────────────────────────

const ART: Partial<Record<SeasonId, string>> = {
  natal: `<svg viewBox="0 0 90 110"><defs><linearGradient id="tg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#22c55e"/><stop offset="1" stop-color="#15803d"/></linearGradient></defs>
    <polygon points="45,6 50,16 61,16 52,23 56,34 45,27 34,34 38,23 29,16 40,16" fill="#facc15" class="art-star"/>
    <path d="M45 22 L70 52 H58 L78 78 H62 L84 104 H6 L28 78 H12 L32 52 H20 Z" fill="url(#tg)"/>
    <path d="M24 70 Q45 80 66 66 M18 94 Q45 104 74 90" stroke="#fde68a" stroke-width="1.4" fill="none" opacity="0.8"/>
    <circle cx="36" cy="58" r="3.2" fill="#ef4444" class="art-tw"/><circle cx="56" cy="62" r="3.2" fill="#3b82f6" class="art-tw d2"/>
    <circle cx="30" cy="86" r="3.4" fill="#facc15" class="art-tw d3"/><circle cx="52" cy="82" r="3.4" fill="#ef4444" class="art-tw d4"/>
    <circle cx="66" cy="96" r="3.4" fill="#a855f7" class="art-tw d2"/><circle cx="42" cy="98" r="3" fill="#22d3ee" class="art-tw"/>
    <rect x="40" y="104" width="10" height="6" fill="#7c2d12"/></svg>`,
  anonovo: `<svg viewBox="0 0 100 100"><g class="art-clink-l"><path d="M30 20 L44 20 L40 50 Q37 58 37 66 L37 84 L45 88 L27 88 L35 84 L35 66 Q35 58 32 50 Z" fill="rgba(253,230,138,0.25)" stroke="#fde68a" stroke-width="1.6"/><path d="M32 30 L42 30 L39.5 48 L34 48 Z" fill="#facc15" opacity="0.8"/></g>
    <g class="art-clink-r"><path d="M56 20 L70 20 L68 50 Q65 58 65 66 L65 84 L73 88 L55 88 L63 84 L63 66 Q63 58 60 50 Z" fill="rgba(253,230,138,0.25)" stroke="#fde68a" stroke-width="1.6"/><path d="M58 30 L68 30 L66.5 48 L61 48 Z" fill="#facc15" opacity="0.8"/></g>
    <g fill="#fde68a" class="art-tw"><circle cx="50" cy="12" r="2"/><circle cx="40" cy="8" r="1.4"/><circle cx="60" cy="8" r="1.4"/></g></svg>`,
  carnaval: `<svg viewBox="0 0 110 80"><path d="M10 30 Q20 12 42 22 Q55 30 68 22 Q90 12 100 30 Q98 52 76 54 Q62 54 55 42 Q48 54 34 54 Q12 52 10 30 Z" fill="#a855f7" stroke="#facc15" stroke-width="2"/>
    <ellipse cx="34" cy="36" rx="9" ry="6" fill="#0b0712"/><ellipse cx="76" cy="36" rx="9" ry="6" fill="#0b0712"/>
    <path d="M100 30 Q112 10 104 0 M98 28 Q116 22 110 8" stroke="#ec4899" stroke-width="3" fill="none" class="art-sway"/>
    <circle cx="20" cy="24" r="2" fill="#facc15"/><circle cx="90" cy="24" r="2" fill="#22c55e"/><circle cx="55" cy="30" r="2.4" fill="#38bdf8"/></svg>`,
  pascoa: `<svg viewBox="0 0 110 80"><ellipse cx="34" cy="44" rx="13" ry="17" fill="#f9a8d4"/><path d="M22 44 Q34 36 46 44" stroke="#fff" stroke-width="2.4" fill="none"/>
    <ellipse cx="58" cy="40" rx="13" ry="18" fill="#a7f3d0"/><path d="M46 38 L52 44 L58 38 L64 44 L70 38" stroke="#fff" stroke-width="2" fill="none"/>
    <ellipse cx="80" cy="46" rx="12" ry="16" fill="#fde68a"/><circle cx="76" cy="42" r="2" fill="#f472b6"/><circle cx="84" cy="50" r="2" fill="#a78bfa"/><circle cx="82" cy="40" r="2" fill="#60a5fa"/>
    <path d="M14 52 H98 L90 76 H22 Z" fill="#b45309"/><path d="M18 60 H94 M20 68 H92" stroke="#92400e" stroke-width="2"/></svg>`,
  junina: `<svg viewBox="0 0 100 100"><path d="M22 92 L78 70 M22 70 L78 92 M30 96 L70 64" stroke="#78350f" stroke-width="7" stroke-linecap="round"/>
    <path d="M50 76 C30 60 42 44 40 28 C52 40 58 30 56 16 C70 34 74 52 62 66 C70 62 72 54 70 48 C80 62 72 80 50 76 Z" fill="#f97316" class="art-flame"/>
    <path d="M50 74 C40 64 46 54 46 46 C54 54 58 48 58 40 C64 52 64 64 50 74 Z" fill="#facc15" class="art-flame d2"/></svg>`,
  halloween: `<svg viewBox="0 0 110 110"><g stroke="rgba(255,255,255,0.35)" stroke-width="1" fill="none">
    <path d="M110 0 L40 70 M110 0 L60 90 M110 0 L80 100 M110 0 L20 40 M110 0 L0 10"/>
    <path d="M86 24 Q90 30 96 28 Q94 34 98 40 M66 30 Q76 40 86 38 Q84 48 92 58 M46 34 Q62 52 74 52 Q72 66 84 80"/></g>
    <g class="art-spider"><line x1="70" y1="0" x2="70" y2="58" stroke="rgba(255,255,255,0.4)" stroke-width="0.8"/><circle cx="70" cy="62" r="5" fill="#0b0712"/><circle cx="70" cy="56" r="3.2" fill="#0b0712"/>
    <path d="M65 60 L58 56 M65 63 L57 64 M65 66 L59 70 M75 60 L82 56 M75 63 L83 64 M75 66 L81 70" stroke="#0b0712" stroke-width="1.4"/></g>
</svg>`,
  aniversario: `<svg viewBox="0 0 100 100"><rect x="18" y="52" width="64" height="34" rx="5" fill="#f9a8d4"/><path d="M18 62 Q26 70 34 62 Q42 70 50 62 Q58 70 66 62 Q74 70 82 62" fill="none" stroke="#fff" stroke-width="3"/>
    <rect x="12" y="84" width="76" height="6" rx="3" fill="#e5e7eb"/><rect x="47" y="34" width="6" height="18" rx="2" fill="#60a5fa"/>
    <path d="M50 20 C45 27 46 32 50 33 C54 32 55 27 50 20 Z" fill="#facc15" class="art-flame"/>
    <g class="art-tw"><circle cx="28" cy="74" r="2" fill="#facc15"/><circle cx="44" cy="76" r="2" fill="#60a5fa"/><circle cx="60" cy="74" r="2" fill="#34d399"/><circle cx="74" cy="76" r="2" fill="#a78bfa"/></g></svg>`,
};

const PUMPKIN = `<svg viewBox="0 0 60 50"><ellipse cx="30" cy="28" rx="28" ry="21" fill="#f97316"/><path d="M30 8 Q28 1 35 -1" stroke="#15803d" stroke-width="3.5" fill="none"/>
  <path d="M17 23 L23 17 L25 25 Z M35 25 L37 17 L43 23 Z M16 33 Q30 45 44 33 L39 35 L35 31 L30 37 L25 31 L21 35 Z" fill="#fde047" class="art-tw"/></svg>`;

/** A small drawn mark for the season (the card on Início, never emoji). */
export function seasonIcon(season: SeasonId, size = 34): HTMLElement {
  const el = h("span", { class: `season-icon art-${season}`, "aria-hidden": "true" });
  el.style.width = `${size}px`;
  el.style.height = `${size}px`;
  el.innerHTML = season === "halloween" ? PUMPKIN : ART[season] ?? "";
  return el;
}

/** The season's ornament for a corner of the panel. */
export function cornerArt(season: SeasonId): HTMLElement | null {
  if (season === "halloween") {
    const wrap = h("div", { "aria-hidden": "true", style: "display:contents" });
    const web = h("div", { class: "season-art art-halloween" });
    web.innerHTML = ART.halloween ?? "";
    const pumpkin = h("div", { class: "season-art art-pumpkin" });
    pumpkin.innerHTML = PUMPKIN;
    wrap.append(web, pumpkin);
    return wrap;
  }
  const svg = ART[season];
  if (!svg) return null;
  const el = h("div", { class: `season-art art-${season}`, "aria-hidden": "true" });
  el.innerHTML = svg;
  return el;
}

// ── The season card on Início ───────────────────────────────────────────────

/** "Faltam 29 dias para o Natal", "Faltam 3h 12min para 2027"… */
export function seasonCountdown(season: SeasonId, now: Date): string {
  const y = now.getFullYear();
  const days = (target: Date) => Math.ceil((target.getTime() - now.getTime()) / 86_400_000);
  /** This year's date, or next year's once it has gone by (a season chosen by hand). */
  const upcoming = (m: number, d: number) => {
    const t = new Date(y, m, d);
    return days(t) < -1 ? new Date(y + 1, m, d) : t;
  };
  switch (season) {
    case "natal": {
      const d = days(upcoming(11, 25));
      if (d > 1) return `Faltam ${d} dias para o Natal`;
      if (d === 1) return "Hoje é véspera de Natal";
      return d === 0 ? "Hoje é Natal" : "Boas festas!";
    }
    case "anonovo": {
      if (now.getMonth() === 0 && now.getDate() <= 2) return `Feliz ${y}!`;
      const next = new Date(y + 1, 0, 1);
      const ms = next.getTime() - now.getTime();
      if (ms < 86_400_000) {
        const hh = Math.floor(ms / 3_600_000);
        const mm = Math.floor((ms % 3_600_000) / 60_000);
        return `Faltam ${hh}h ${String(mm).padStart(2, "0")}min para ${next.getFullYear()}`;
      }
      return `Faltam ${days(next)} dias para ${next.getFullYear()}`;
    }
    case "carnaval": return "É Carnaval!";
    case "pascoa": return "Semana de Páscoa";
    case "junina": {
      const d = days(upcoming(5, 24));
      return d > 0 ? `Arraiá! Faltam ${d} dias para o São João` : d === 0 ? "Hoje é São João!" : "Arraiá do Jarvis";
    }
    case "halloween": {
      const d = days(upcoming(9, 31));
      return d > 0 ? `Faltam ${d} dias para o Halloween` : "Hoje é Halloween!";
    }
    case "aniversario": return "Hoje é o seu dia!";
  }
}
