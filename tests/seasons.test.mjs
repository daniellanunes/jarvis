// Seasonal looks (src/jarvis/seasons.ts): the right season on the right days.

import { test } from "node:test";
import assert from "node:assert/strict";
import { activeSeason, parseBirthday, seasonGreeting, seasonOn } from "../src/jarvis/seasons.ts";

const day = (y, m, d) => new Date(y, m - 1, d, 12);

test("the calendar's seasons", () => {
  assert.equal(seasonOn(day(2026, 12, 1)), "natal");
  assert.equal(seasonOn(day(2026, 12, 26)), "natal");
  assert.equal(seasonOn(day(2026, 12, 27)), "anonovo");
  assert.equal(seasonOn(day(2027, 1, 2)), "anonovo");
  assert.equal(seasonOn(day(2027, 1, 3)), null);
  assert.equal(seasonOn(day(2026, 10, 31)), "halloween");
  assert.equal(seasonOn(day(2026, 10, 24)), null);
  assert.equal(seasonOn(day(2026, 6, 15)), "junina");
  assert.equal(seasonOn(day(2026, 11, 25)), null);
});

test("Carnival and Easter move with Easter", () => {
  // Easter 2027: March 28 → Carnival Tuesday Feb 9.
  assert.equal(seasonOn(day(2027, 2, 9)), "carnaval");
  assert.equal(seasonOn(day(2027, 2, 6)), "carnaval");
  assert.equal(seasonOn(day(2027, 2, 10)), "carnaval");
  assert.equal(seasonOn(day(2027, 2, 11)), null);
  assert.equal(seasonOn(day(2027, 3, 26)), "pascoa");
  assert.equal(seasonOn(day(2027, 3, 29)), "pascoa");
  assert.equal(seasonOn(day(2027, 3, 30)), null);
});

test("the birthday wins over the rest", () => {
  assert.deepEqual(parseBirthday("25/12"), [25, 12]);
  assert.deepEqual(parseBirthday(" 7-3 "), [7, 3]);
  assert.equal(parseBirthday("32/1"), null);
  assert.equal(parseBirthday("amanhã"), null);
  assert.equal(seasonOn(day(2026, 12, 25), "25/12"), "aniversario");
  assert.equal(seasonOn(day(2026, 8, 7), "07/08"), "aniversario");
});

test("greetings only on the special days", () => {
  assert.equal(seasonGreeting(day(2026, 12, 25), "natal", "Bia"), "Feliz Natal, Bia! 🎄");
  assert.equal(seasonGreeting(day(2026, 12, 10), "natal", "Bia"), null);
  assert.equal(seasonGreeting(day(2027, 1, 1), "anonovo", ""), "Feliz Ano Novo! 🎆");
  assert.equal(seasonGreeting(day(2027, 3, 28), "pascoa", "Bia"), "Feliz Páscoa, Bia! 🐰");
  assert.equal(seasonGreeting(day(2026, 8, 7), "aniversario", "Bia"), "Feliz aniversário, Bia! 🎂");
});

test("the setting: auto, off, or a season by hand", () => {
  assert.equal(activeSeason("auto", day(2026, 12, 10)), "natal");
  assert.equal(activeSeason(undefined, day(2026, 12, 10)), "natal");
  assert.equal(activeSeason("off", day(2026, 12, 10)), null);
  assert.equal(activeSeason("halloween", day(2026, 3, 3)), "halloween");
  assert.equal(activeSeason("qualquer", day(2026, 3, 3)), null);
});
