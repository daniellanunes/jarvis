// Renders an animated page of marketing/ into an MP4. Every frame is drawn at
// its exact time (setT), then encoded with H.264.
//
//   node marketing/render.mjs                     → out/jarvis-reels.mp4 (Instagram, 1080×1920)
//   node marketing/render.mjs linkedin            → out/jarvis-linkedin.mp4 (LinkedIn, 1080×1350)
//   node marketing/render.mjs linkedin --preview  → a few frames only, to check the look
//
// Needs Chrome or Edge, and ffmpeg (or `pip install imageio-ffmpeg`).

import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, "out");
const VIDEOS = {
  instagram: { page: "instagram.html", w: 1080, h: 1920, duration: 28.5, out: "jarvis-reels.mp4",
    picks: [0.9, 2.4, 5.2, 8.4, 10.4, 13.2, 16.2, 18.6, 21.4, 24.2, 27.4] },
  linkedin: { page: "linkedin.html", w: 1080, h: 1350, duration: 50.4, out: "jarvis-linkedin.mp4",
    picks: [2.2, 6.0, 9.4, 12.6, 16.5, 22.2, 25.5, 31.0, 35.4, 41.6, 48.9] },
};
const which = process.argv.slice(2).find((a) => !a.startsWith("--")) ?? "instagram";
const VIDEO = VIDEOS[which];
if (!VIDEO) throw new Error(`Vídeo desconhecido: ${which} (${Object.keys(VIDEOS).join(", ")})`);
const FRAMES = join(OUT, `frames-${which}`);
const FPS = 30;
const DURATION = VIDEO.duration;
const W = VIDEO.w;
const H = VIDEO.h;
const PORT = 9555;
const preview = process.argv.includes("--preview");

const chrome = [
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
].find(existsSync);
if (!chrome) throw new Error("Chrome/Edge não encontrado");

function ffmpegPath() {
  try {
    return execFileSync("python", ["-c", "import imageio_ffmpeg as f; print(f.get_ffmpeg_exe())"]).toString().trim();
  } catch {
    return "ffmpeg";
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  mkdirSync(FRAMES, { recursive: true });
  for (const f of readdirSync(FRAMES)) rmSync(join(FRAMES, f), { force: true });

  const browser = spawn(
    chrome,
    [
      "--headless=new",
      "--disable-gpu",
      "--hide-scrollbars",
      "--allow-file-access-from-files",
      `--remote-debugging-port=${PORT}`,
      `--user-data-dir=${join(tmpdir(), `jarvis-reels-${Date.now()}`)}`,
      `--window-size=${W},${H}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  let targets;
  for (let i = 0; i < 50 && !targets; i++) {
    await sleep(200);
    targets = await fetch(`http://127.0.0.1:${PORT}/json/list`).then((r) => r.json()).catch(() => null);
  }
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, (msg) => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = (expression) => send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });

  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", { width: W, height: H, deviceScaleFactor: 1, mobile: false });
  await send("Page.navigate", { url: pathToFileURL(join(HERE, VIDEO.page)).href });
  for (let i = 0; i < 100; i++) {
    await sleep(100);
    const r = await evaluate("typeof window.ready !== 'undefined'").catch(() => null);
    if (r?.result?.value) break;
  }
  await evaluate("window.ready");
  await sleep(300);

  const total = Math.round(DURATION * FPS);
  const picks = preview ? VIDEO.picks.map((s) => Math.round(s * FPS)) : null;
  const frames = picks ?? Array.from({ length: total }, (_, i) => i);
  let n = 0;
  for (const f of frames) {
    await evaluate(`setT(${f / FPS})`);
    const shot = await send("Page.captureScreenshot", { format: preview ? "png" : "jpeg", quality: 95, captureBeyondViewport: false });
    const name = preview ? `preview-${String(f).padStart(4, "0")}.png` : `${String(n).padStart(5, "0")}.jpg`;
    writeFileSync(join(FRAMES, name), Buffer.from(shot.data, "base64"));
    n++;
    if (n % 120 === 0) console.log(`${n}/${frames.length} frames`);
  }
  ws.close();
  browser.kill();

  if (preview) {
    console.log(`preview: ${n} frames in ${FRAMES}`);
    return;
  }
  const out = join(OUT, VIDEO.out);
  execFileSync(
    ffmpegPath(),
    [
      "-y", "-framerate", String(FPS), "-i", join(FRAMES, "%05d.jpg"),
      // A silent stereo track: some apps handle audio-less videos badly.
      "-f", "lavfi", "-i", "anullsrc=channel_layout=stereo:sample_rate=44100",
      "-shortest", "-c:v", "libx264", "-profile:v", "high", "-preset", "slow", "-crf", "17",
      // JPEG frames are full range: convert to the standard (TV) range Instagram expects.
      "-vf", "scale=in_range=pc:out_range=tv,format=yuv420p", "-color_range", "tv", "-colorspace", "bt709",
      "-color_primaries", "bt709", "-color_trc", "bt709", "-r", String(FPS), "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart", out,
    ],
    { stdio: "inherit" },
  );
  console.log(`ok: ${out}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
