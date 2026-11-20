// Dev harness: every outfit drawn large on a plain Mochi, from three angles
// (turned left, facing, turned right), to check the drawings follow the head.
// ?finish=metallic&color=%23f43f5e paints the body with a finish instead.

import { bodyOutline, drawOutfitBehind, drawOutfitFront, eyeFrames, makeHead } from "../src/mochi/outfits";
import { OUTFIT_KEYS, OUTFIT_SELECTIONS, type Outfit } from "../src/mochi/wardrobe";
import { hexToRGB, paintBodyFinish, type BodyFinish } from "../src/mochi/engine";

const params = new URLSearchParams(location.search);
const only = params.get("only")?.split(",");
const finish = (params.get("finish") ?? "solid") as BodyFinish;
const color = params.get("color") ?? "#f43f5e";
const R = 34;
const yaws = [-0.55, 0, 0.55];

const grid = document.getElementById("grid")!;
const outfits = OUTFIT_SELECTIONS.filter((o) => o !== "auto" && (!only || only.includes(o))) as Outfit[];

for (const outfit of outfits) {
  const cell = document.createElement("div");
  cell.className = "cell";
  const canvas = document.createElement("canvas");
  const W = 330;
  const Hh = 150;
  const dpr = 2;
  canvas.width = W * dpr;
  canvas.height = Hh * dpr;
  canvas.style.width = `${W}px`;
  canvas.style.height = `${Hh}px`;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  yaws.forEach((yaw, i) => {
    const H = makeHead(R, yaw, 0);
    ctx.save();
    ctx.translate(55 + i * 110, 92);
    drawOutfitBehind(ctx, outfit, H, { presence: 1, morph: 0 });
    const body = bodyOutline(H.rx, H.ry);
    paintBodyFinish(ctx, body, hexToRGB(color), finish, R, H.rx, H.ry);
    ctx.save();
    ctx.clip(body);
    // Same rule as the engine: neon eyes glow in the body colour.
    ctx.fillStyle = finish === "neon" ? color : "rgb(26,20,18)";
    for (const e of eyeFrames(H)) {
      if (!e.visible) continue;
      ctx.beginPath();
      ctx.ellipse(e.x, e.y, (e.w / 2) * e.fx, (e.h / 2) * e.fy, 0, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
    drawOutfitFront(ctx, outfit, H, { presence: 1, morph: 0 });
    ctx.restore();
  });
  const label = document.createElement("div");
  label.textContent = `${outfit} — ${OUTFIT_KEYS[outfit]}`;
  cell.append(canvas, label);
  grid.append(cell);
}
