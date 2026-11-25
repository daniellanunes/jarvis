// Seasonal looks for the notch and the mascot: Christmas in December, New
// Year, Carnival, Easter, Festa Junina, Halloween and the user's birthday —
// switched on and off by the calendar (Ajustes → Notch → "Temas da época").
// No DOM here: the calendar is tested in Node.

import { easterDate, type Outfit } from "../mochi/wardrobe";

export type SeasonId = "aniversario" | "anonovo" | "natal" | "carnaval" | "pascoa" | "halloween" | "junina";

export interface SeasonInfo {
  id: SeasonId;
  name: string;
  emoji: string;
  /** What the mascot wears while it lasts. */
  outfit: Outfit;
}

export const SEASONS: SeasonInfo[] = [
  { id: "natal", name: "Natal", emoji: "🎄", outfit: "santaHat" },
  { id: "anonovo", name: "Ano Novo", emoji: "🎆", outfit: "partyHat" },
  { id: "carnaval", name: "Carnaval", emoji: "🎭", outfit: "mask" },
  { id: "pascoa", name: "Páscoa", emoji: "🐰", outfit: "bunnyEars" },
  { id: "junina", name: "Festa Junina", emoji: "🌽", outfit: "cowboyHat" },
  { id: "halloween", name: "Halloween", emoji: "🎃", outfit: "witchHat" },
  { id: "aniversario", name: "Aniversário", emoji: "🎂", outfit: "partyHat" },
];

export function seasonInfo(id: SeasonId): SeasonInfo {
  return SEASONS.find((s) => s.id === id) ?? SEASONS[0];
}

const DAY_MS = 86_400_000;

/** Days from Easter Sunday to `date` (calendar days, safe across DST). */
function fromEaster(date: Date): number {
  const y = date.getFullYear();
  const [em, ed] = easterDate(y);
  return Math.round((Date.UTC(y, date.getMonth(), date.getDate()) - Date.UTC(y, em - 1, ed)) / DAY_MS);
}

/** "25/12" or "25-12" → [day, month]; anything else → null. */
export function parseBirthday(raw: string | undefined): [number, number] | null {
  const m = (raw ?? "").trim().match(/^(\d{1,2})\s*[/.-]\s*(\d{1,2})$/);
  if (!m) return null;
  const d = Number(m[1]);
  const mo = Number(m[2]);
  return d >= 1 && d <= 31 && mo >= 1 && mo <= 12 ? [d, mo] : null;
}

/**
 * The season of `date` in Brazil's calendar, or null. When two overlap, the
 * more personal / shorter one wins: birthday > New Year > Christmas >
 * Carnival > Easter > Halloween > Festa Junina.
 */
export function seasonOn(date: Date, birthday?: string): SeasonId | null {
  const d = date.getDate();
  const m = date.getMonth() + 1;
  const b = parseBirthday(birthday);
  if (b && b[0] === d && b[1] === m) return "aniversario";
  if ((m === 12 && d >= 27) || (m === 1 && d <= 2)) return "anonovo";
  if (m === 12 && d <= 26) return "natal";
  const e = fromEaster(date);
  // Saturday before Carnival Tuesday (Easter − 47) to Ash Wednesday.
  if (e >= -50 && e <= -46) return "carnaval";
  // Good Friday to Easter Monday.
  if (e >= -2 && e <= 1) return "pascoa";
  if (m === 10 && d >= 25) return "halloween";
  if (m === 6) return "junina";
  return null;
}

/** The special day's greeting (said once that day), or null on the other days. */
export function seasonGreeting(date: Date, id: SeasonId | null, name: string): string | null {
  const d = date.getDate();
  const m = date.getMonth() + 1;
  const who = name.trim() ? `, ${name.trim()}` : "";
  switch (id) {
    case "aniversario":
      return `Feliz aniversário${who}! 🎂`;
    case "natal":
      if (m === 12 && d === 24) return `Feliz véspera de Natal${who}! 🎄`;
      if (m === 12 && d === 25) return `Feliz Natal${who}! 🎄`;
      return null;
    case "anonovo":
      if (m === 12 && d === 31) return `Último dia do ano${who}! 🎆`;
      if (m === 1 && d === 1) return `Feliz Ano Novo${who}! 🎆`;
      return null;
    case "pascoa":
      return fromEaster(date) === 0 ? `Feliz Páscoa${who}! 🐰` : null;
    case "carnaval":
      return fromEaster(date) === -47 ? `Bom Carnaval${who}! 🎭` : null;
    case "halloween":
      return m === 10 && d === 31 ? `Feliz Halloween${who}! 🎃` : null;
    case "junina":
      return m === 6 && d === 24 ? `Viva São João${who}! 🌽` : null;
    default:
      return null;
  }
}

/**
 * The season to show: the setting is "auto" (follow the calendar), "off", or
 * a season chosen by hand (to try it or keep it).
 */
export function activeSeason(setting: string | undefined, date: Date, birthday?: string): SeasonId | null {
  const s = (setting ?? "auto").trim();
  if (s === "off") return null;
  if (SEASONS.some((x) => x.id === s)) return s as SeasonId;
  return seasonOn(date, birthday);
}
