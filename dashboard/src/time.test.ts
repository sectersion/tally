import { expect, test } from "bun:test";
import { dayEdges, formatDuration } from "./time";

const H = 3_600_000;
const NY = "America/New_York";

test("dayEdges returns a 23 hour day across a spring-forward change", () => {
  const e = dayEdges(Date.parse("2026-03-08T18:00:00Z"), 3, NY);
  expect(e.length).toBe(4);
  // last day (03-08) is 23 h; the edge after it is 03-09 00:00 EDT
  expect(e[2] - e[1]).toBe(24 * H);
  expect(e[3] - e[2]).toBe(23 * H);
});
test("dayEdges returns a 25 hour day across a fall-back change", () => {
  const e = dayEdges(Date.parse("2026-11-01T18:00:00Z"), 2, NY);
  expect(e[2] - e[1]).toBe(25 * H);
});
test("dayEdges are strictly ascending and have days + 1 entries", () => {
  const e = dayEdges(Date.now(), 28);
  expect(e.length).toBe(29);
  for (let i = 1; i < e.length; i++) expect(e[i]).toBeGreaterThan(e[i - 1]);
});
test("edges for Pacific/Kiritimati (UTC+14) and Pacific/Pago_Pago (UTC-11) still start at local midnight", () => {
  const now = Date.parse("2026-06-15T12:00:00Z");
  const hm = (t: number, tz: string) =>
    new Intl.DateTimeFormat("en-GB", { timeZone: tz, hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(t));
  for (const tz of ["Pacific/Kiritimati", "Pacific/Pago_Pago"]) {
    for (const e of dayEdges(now, 3, tz)) expect(hm(e, tz)).toBe("00:00");
  }
});
test("formatDuration", () => {
  expect(formatDuration(42)).toBe("42 s");
  expect(formatDuration(3900)).toBe("1 h 05 min");
  expect(formatDuration(0)).toBe("0 s");
});
