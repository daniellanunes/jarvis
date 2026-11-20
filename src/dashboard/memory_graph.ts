// Obsidian-style interactive Neural Mesh graph rendered on HTML5 Canvas.
// Supports pan, zoom, force-directed springs along real edges, and neighbor highlighting.

import type { MemoryEdge, MemoryNode } from "../core/bridge";

interface GraphNode {
  data: MemoryNode;
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
}

export const CATEGORY_META: Record<string, { color: string; label: string }> = {
  user: { color: "#ffffff", label: "Você" },
  pref: { color: "#eab308", label: "Hub / Categoria" },
  project: { color: "#8b5cf6", label: "Projeto" },
  tech: { color: "#06b6d4", label: "Tecnologia" },
  person: { color: "#f43f5e", label: "Pessoa" },
  fact: { color: "#84cc16", label: "Memória / Fato" },
};

export function renderMemoryGraph(
  canvas: HTMLCanvasElement,
  memories: MemoryNode[],
  edges: MemoryEdge[] = [],
  filter: string,
  onSelect: (node: MemoryNode | null) => void,
): () => void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return () => {};

  const dpr = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || 620;
  const height = canvas.clientHeight || 310;
  canvas.width = width * dpr;
  canvas.height = height * dpr;
  ctx.scale(dpr, dpr);

  const cx = width / 2;
  const cy = height / 2;
  const q = filter.trim().toLowerCase();

  const visible = memories.filter(
    (m) =>
      m.category === "user" ||
      !q ||
      m.label.toLowerCase().includes(q) ||
      (m.notes ?? "").toLowerCase().includes(q) ||
      m.category.toLowerCase().includes(q),
  );
  const visibleIds = new Set(visible.map((m) => m.id));

  // Build adjacency list from real edges (fallback to root connection for orphans)
  const validEdges: { a: string; b: string; label: string }[] = [];
  const connected = new Set<string>();
  for (const e of edges) {
    if (visibleIds.has(e.from) && visibleIds.has(e.to)) {
      validEdges.push({ a: e.from, b: e.to, label: e.label });
      connected.add(e.from);
      connected.add(e.to);
    }
  }

  const rootId = visible.find((m) => m.category === "user")?.id ?? visible[0]?.id;
  for (const m of visible) {
    if (m.id !== rootId && !connected.has(m.id) && rootId) {
      validEdges.push({ a: rootId, b: m.id, label: "" });
    }
  }

  const nodes: GraphNode[] = visible.map((m, i) => {
    if (m.category === "user") {
      return { data: m, x: cx, y: cy, vx: 0, vy: 0, radius: 11 };
    }
    const ring = m.category === "pref" ? 85 : m.category === "project" ? 145 : 195;
    const angle = (i / Math.max(1, visible.length - 1)) * Math.PI * 2;
    return {
      data: m,
      x: cx + Math.cos(angle) * ring + ((i * 17) % 19) - 9,
      y: cy + Math.sin(angle) * ring + ((i * 23) % 19) - 9,
      vx: 0,
      vy: 0,
      radius: m.category === "pref" ? 8 : m.category === "project" ? 7 : 5.5,
    };
  });

  const byId = new Map<string, GraphNode>(nodes.map((n) => [n.data.id, n]));
  const root = rootId ? byId.get(rootId) ?? null : null;

  let panX = 0;
  let panY = 0;
  let zoom = 1;
  let draggingNode: GraphNode | null = null;
  let panning = false;
  let panStartX = 0;
  let panStartY = 0;
  let hovered: GraphNode | null = null;
  let selected: GraphNode | null = null;
  let raf = 0;

  function step() {
    if (!ctx) return;

    // Force-directed simulation
    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      if (a === draggingNode) continue;

      if (a === root) {
        a.vx += (cx - a.x) * 0.05;
        a.vy += (cy - a.y) * 0.05;
      } else {
        // Gentle centering gravity
        a.vx += (cx - a.x) * 0.0012;
        a.vy += (cy - a.y) * 0.0012;
      }

      for (let j = i + 1; j < nodes.length; j++) {
        const b = nodes[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const d2 = dx * dx + dy * dy + 1;
        if (d2 < 12000) {
          const rep = 220 / d2;
          const fx = dx * rep;
          const fy = dy * rep;
          if (a !== draggingNode) {
            a.vx += fx;
            a.vy += fy;
          }
          if (b !== draggingNode) {
            b.vx -= fx;
            b.vy -= fy;
          }
        }
      }
    }

    // Spring forces along edges
    for (const edge of validEdges) {
      const a = byId.get(edge.a);
      const b = byId.get(edge.b);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d = Math.hypot(dx, dy) || 1;
      const ideal = a === root || b === root ? 78 : 68;
      const f = (d - ideal) * 0.012;
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      if (a !== draggingNode && a !== root) {
        a.vx += fx;
        a.vy += fy;
      }
      if (b !== draggingNode && b !== root) {
        b.vx -= fx;
        b.vy -= fy;
      }
    }

    for (const n of nodes) {
      if (n === draggingNode) continue;
      n.vx *= 0.82;
      n.vy *= 0.82;
      n.x += n.vx;
      n.y += n.vy;
    }

    // Render
    ctx.clearRect(0, 0, width, height);
    ctx.save();
    ctx.translate(cx + panX, cy + panY);
    ctx.scale(zoom, zoom);
    ctx.translate(-cx, -cy);

    const activeNode = hovered ?? selected;
    const neighborIds = new Set<string>();
    if (activeNode) {
      neighborIds.add(activeNode.data.id);
      for (const e of validEdges) {
        if (e.a === activeNode.data.id) neighborIds.add(e.b);
        if (e.b === activeNode.data.id) neighborIds.add(e.a);
      }
    }

    // Draw edges
    for (const e of validEdges) {
      const a = byId.get(e.a);
      const b = byId.get(e.b);
      if (!a || !b) continue;
      const isHighlighted =
        activeNode && (e.a === activeNode.data.id || e.b === activeNode.data.id);
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = isHighlighted
        ? "rgba(163, 230, 53, 0.78)"
        : activeNode
          ? "rgba(255, 255, 255, 0.05)"
          : "rgba(255, 255, 255, 0.13)";
      ctx.lineWidth = isHighlighted ? 1.7 : 0.9;
      ctx.stroke();
    }

    // Draw nodes
    for (const n of nodes) {
      const meta = CATEGORY_META[n.data.category] ?? { color: "#9ca3af", label: "Nota" };
      const dimmed = activeNode && !neighborIds.has(n.data.id);
      const isFocus = n === hovered || n === selected;

      ctx.globalAlpha = dimmed ? 0.28 : 1;
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius + (isFocus ? 2 : 0), 0, Math.PI * 2);
      ctx.fillStyle = meta.color;
      ctx.shadowColor = meta.color;
      ctx.shadowBlur = isFocus ? 16 : 7;
      ctx.fill();
      ctx.shadowBlur = 0;

      ctx.font =
        n === root
          ? "600 11.5px -apple-system, Segoe UI, sans-serif"
          : isFocus
            ? "600 10.5px -apple-system, Segoe UI, sans-serif"
            : "10px -apple-system, Segoe UI, sans-serif";
      ctx.fillStyle = isFocus ? "#ffffff" : "#cbd5e1";
      ctx.textAlign = "center";
      ctx.fillText(n.data.label, n.x, n.y + n.radius + 12);
    }
    ctx.globalAlpha = 1;
    ctx.restore();

    raf = requestAnimationFrame(step);
  }

  function screenToWorld(sx: number, sy: number) {
    return {
      x: (sx - cx - panX) / zoom + cx,
      y: (sy - cy - panY) / zoom + cy,
    };
  }

  function hitTest(sx: number, sy: number): GraphNode | null {
    const { x, y } = screenToWorld(sx, sy);
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      if (Math.hypot(n.x - x, n.y - y) <= n.radius + 7) return n;
    }
    return null;
  }

  function localCoords(e: MouseEvent) {
    const r = canvas.getBoundingClientRect();
    return { sx: e.clientX - r.left, sy: e.clientY - r.top };
  }

  const onDown = (e: MouseEvent) => {
    const { sx, sy } = localCoords(e);
    const hit = hitTest(sx, sy);
    if (hit) {
      draggingNode = hit;
      selected = hit;
      onSelect(hit.data);
    } else {
      panning = true;
      panStartX = e.clientX - panX;
      panStartY = e.clientY - panY;
      selected = null;
      onSelect(null);
    }
  };

  const onMove = (e: MouseEvent) => {
    const { sx, sy } = localCoords(e);
    if (draggingNode) {
      const w = screenToWorld(sx, sy);
      draggingNode.x = w.x;
      draggingNode.y = w.y;
      draggingNode.vx = 0;
      draggingNode.vy = 0;
    } else if (panning) {
      panX = e.clientX - panStartX;
      panY = e.clientY - panStartY;
    } else {
      hovered = hitTest(sx, sy);
      canvas.style.cursor = hovered ? "pointer" : "grab";
    }
  };

  const onUp = () => {
    draggingNode = null;
    panning = false;
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.12 : 0.9;
    zoom = Math.max(0.45, Math.min(2.6, zoom * factor));
  };

  canvas.addEventListener("mousedown", onDown);
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
  canvas.addEventListener("wheel", onWheel, { passive: false });
  raf = requestAnimationFrame(step);

  return () => {
    cancelAnimationFrame(raf);
    canvas.removeEventListener("mousedown", onDown);
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    canvas.removeEventListener("wheel", onWheel);
  };
}
