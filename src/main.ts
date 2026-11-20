// Entry point for the Jarvis Notch (reuses OpenJarvis's Mochi mascot + sounds + Tauri bridge).

import { startJarvisNotch } from "./jarvis/notch_app";

const root = document.getElementById("root");
if (root) {
  void startJarvisNotch(root);
}
