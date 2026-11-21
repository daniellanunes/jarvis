// Voice: the wake word and Whisper's "noise" (src/jarvis/voice.ts).

import { test } from "node:test";
import assert from "node:assert/strict";
import { asksBack, commandAfterWake, isNoise, matchWakeWord, speakable } from "../src/jarvis/voice.ts";

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

test("a reply that ends with a question opens the mic for the answer", () => {
  for (const r of ["Quer que eu toque a próxima?", "Abri o Spotify. Qual playlist você quer? 🎵", 'Posso mandar "oi" pra Ana?"', "Confirmo o horário das 15h? **"]) {
    assert.equal(asksBack(r), true, r);
  }
  for (const r of ["Pronto, Spotify aberto!", "Você perguntou se dava? Dá sim, já abri.", "Tocando agora.", ""]) {
    assert.equal(asksBack(r), false, r);
  }
});

test("a question followed by a short invitation also waits for the answer", () => {
  assert.equal(asksBack("Beleza! Quer focar em textos ou documentação? É só mandar."), true);
  assert.equal(asksBack("Qual playlist? Me diga."), true);
  assert.equal(asksBack("Você perguntou se dava? Dá sim, já abri o Spotify e coloquei a sua playlist."), false);
});

test("saying Jarvis again at the start of the answer is dropped", () => {
  const raw = "Jarvis. Me faça outra pergunta agora de novo.";
  assert.equal(commandAfterWake(raw, matchWakeWord(raw)), "Me faça outra pergunta agora de novo.");
  const twice = "Jarvis, Jarvis, toca Coldplay";
  assert.equal(commandAfterWake(twice, matchWakeWord(twice)), "toca Coldplay");
  const inside = "Jarvis, manda pro Jarvis do trabalho";
  assert.equal(commandAfterWake(inside, matchWakeWord(inside)), "manda pro Jarvis do trabalho");
});

test("the voice does not read markdown, links or emoji", () => {
  assert.equal(speakable("**Pronto!** Abri o *Spotify* 🎵"), "Pronto! Abri o Spotify");
  assert.equal(speakable("## Sua agenda\n- Reunião às 10h\n- Dentista às 15h"), "Sua agenda. Reunião às 10h. Dentista às 15h");
  assert.equal(speakable("Veja em [o site](https://exemplo.com) ou https://a.com/x"), "Veja em o site ou o link");
  assert.equal(speakable("Seu projeto em `D:\\Projetos\\jarvis` está ok."), "Seu projeto em Projetos jarvis está ok.");
  assert.equal(speakable("Use o código:\n```js\nconst a = 1;\n```\nPronto."), "Use o código: Pronto.");
  assert.equal(speakable("5 * 3 = 15"), "5 3 = 15");
});
