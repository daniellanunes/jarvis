// Builds a signed release and the update feed.
//
//   npm run release
//
// 1. Reads the version from src-tauri/tauri.conf.json.
// 2. Signs with the updater key (TAURI_SIGNING_PRIVATE_KEY, or the file in
//    JARVIS_UPDATER_KEY, default ~/.tauri/jarvis-updater.key — never in the repo).
// 3. Runs `tauri build` → target/release/bundle/nsis/Jarvis_<v>_x64-setup.exe (+ .sig).
// 4. Writes latest.json next to it, with the notes from docs/releases/v<v>.md.
//
// Upload the .exe and latest.json to a GitHub release tagged v<v>: installed
// copies of Jarvis find it at releases/latest/download/latest.json.

import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = "daniellanunes/jarvis";

const conf = JSON.parse(readFileSync(join(ROOT, "src-tauri/tauri.conf.json"), "utf8"));
const version = conf.version;
const product = conf.productName;

const env = { ...process.env };
if (!env.TAURI_SIGNING_PRIVATE_KEY) {
  const keyFile = env.JARVIS_UPDATER_KEY ?? join(homedir(), ".tauri", "jarvis-updater.key");
  if (!existsSync(keyFile)) {
    console.error(`Chave de assinatura não encontrada: ${keyFile}\nDefina TAURI_SIGNING_PRIVATE_KEY ou JARVIS_UPDATER_KEY.`);
    process.exit(1);
  }
  env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyFile, "utf8").trim();
}
env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";

console.log(`Gerando ${product} ${version} assinado…`);
execSync("npx tauri build", { cwd: ROOT, stdio: "inherit", env });

const dir = join(ROOT, "target/release/bundle/nsis");
const exe = `${product}_${version}_x64-setup.exe`;
const sigFile = join(dir, `${exe}.sig`);
if (!existsSync(sigFile)) {
  console.error(`Assinatura não gerada: ${sigFile}`);
  process.exit(1);
}

const notesFile = join(ROOT, `docs/releases/v${version}.md`);
const notes = existsSync(notesFile)
  ? readFileSync(notesFile, "utf8")
      .split("\n")
      .filter((l) => !l.startsWith("# "))
      .join("\n")
      .trim()
  : `${product} ${version}`;

const feed = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(sigFile, "utf8").trim(),
      url: `https://github.com/${REPO}/releases/download/v${version}/${exe}`,
    },
  },
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(feed, null, 2));

console.log(`
Pronto. Crie a release v${version} em https://github.com/${REPO}/releases/new
e anexe estes dois arquivos de ${dir}:
  - ${exe}
  - latest.json
Texto da release: docs/releases/v${version}.md
`);
