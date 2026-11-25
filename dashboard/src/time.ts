const parts = (t: number, tz?: string) => {
  const f = new Intl.DateTimeFormat("en-US", {
    timeZone: tz, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric",
    hour: "numeric", minute: "numeric", second: "numeric",
  });
  const o: Record<string, number> = {};
  for (const p of f.formatToParts(new Date(t))) if (p.type !== "literal") o[p.type] = Number(p.value);
  return o;
};
/** tz offset in ms at instant t (local wall clock minus UTC). */
const offset = (t: number, tz?: string) => {
  const p = parts(t, tz);
  return Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second) - Math.floor(t / 1000) * 1000;
};

/**
 * The `days + 1` instants of local midnight, ending with the midnight after `now`'s day.
 * Built from calendar dates, never by adding 86,400,000 (DST days are 23 or 25 hours).
 * ponytail: a zone whose midnight does not exist on a DST day lands an hour off; none are in use today.
 */
export function dayEdges(now: number, days: number, tz?: string): number[] {
  const p = parts(now, tz);
  const out: number[] = [];
  for (let k = -(days - 1); k <= 1; k++) {
    const guess = Date.UTC(p.year, p.month - 1, p.day + k);
    out.push(guess - offset(guess - offset(guess, tz), tz));
  }
  return out;
}

export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`;
}

export const formatClock = (ts: number): string =>
  new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });

export const dayLabel = (edge: number): string =>
  new Date(edge + 3_600_000 * 2).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
