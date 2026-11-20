// Voice: the wake word and Whisper's "noise" (src/jarvis/voice.ts).

import { test } from "node:test";
import assert from "node:assert/strict";
import { isNoise, matchWakeWord } from "../src/jarvis/voice.ts";

const after = (raw) => {
  const m = matchWakeWord(raw);
  return m ? raw.slice(m.endIndex).replace(/^[,.\s:!?-]+/, "").trim() : null;
};

test("the command after the wake word is what runs", () => {
  assert.equal(after("Jarvis, toca Dan FM do The Weeknd."), "toca Dan FM do The Weeknd.");
  assert.equal(after("Ei Jarvis pausa a música"), "pausa a música");
  assert.equal(after("Ok, Járvis. Volume 30"), "Volume 30");
});

test("common mishearings of Jarvis still wake it", () => {
  for (const heard of ["Javis, abre o YouTube", "Jarbas abre o YouTube", "Jervis, abre o YouTube", "Charvis abre o YouTube"]) {
    assert.equal(after(heard), "abre o YouTube", heard);
  }
});

test("just the wake word leaves no command (the next sentence is it)", () => {
  assert.equal(after("Jarvis."), "");
  assert.equal(after("Ei, Jarvis!"), "");
});

test("sentences without the wake word do not run", () => {
  assert.equal(after("toca alguma coisa aí"), null);
  assert.equal(after("o jardim está bonito"), null);
});

test("Whisper's noise is never a command", () => {
  for (const n of ["Obrigado.", "Legendas pela comunidade Amara.org", "Tchau!", "Hum", "", " . "]) {
    assert.equal(isNoise(n), true, n);
  }
  assert.equal(isNoise("toca Dawn FM"), false);
  assert.equal(isNoise("volume 30"), false);
});
