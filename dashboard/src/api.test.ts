import { expect, test } from "bun:test";
import { Api, ApiError, poll } from "./api";

test("api maps a 503 to ApiError with status 503", async () => {
  const api = new Api("", async () => new Response(JSON.stringify({ error: "no database yet" }), { status: 503 }));
  const e = await api.status().catch((x) => x);
  expect(e).toBeInstanceOf(ApiError);
  expect(e.status).toBe(503);
  expect(e.message).toBe("no database yet");
});

function sched() {
  const q: { fn: () => void; ms: number }[] = [];
  return { q, schedule: (fn: () => void, ms: number) => { const j = { fn, ms }; q.push(j); return () => { q.splice(q.indexOf(j), 1); }; } };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

test("poll backs off after failures, resets after success, and does not fire while hidden", async () => {
  const s = sched();
  let fail = true, calls = 0;
  poll(async () => { calls++; if (fail) throw new Error("x"); return 1; }, { onResult() {}, onError() {}, schedule: s.schedule, intervalMs: 100, maxBackoffMs: 300 });
  await flush();
  expect(s.q.map((j) => j.ms)).toEqual([200]);
  s.q.shift()!.fn(); await flush();
  expect(s.q[0].ms).toBe(300); // capped
  fail = false;
  s.q.shift()!.fn(); await flush();
  expect(s.q[0].ms).toBe(100); // reset
  Object.defineProperty(document, "hidden", { value: true, configurable: true });
  const before = calls;
  s.q.shift()!.fn(); await flush();
  expect(calls).toBe(before);
  expect(s.q.length).toBe(1); // still scheduled
  Object.defineProperty(document, "hidden", { value: false, configurable: true });
  s.q.shift()!.fn(); await flush();
  expect(calls).toBe(before + 1);
});

test("poll's stop function ends all further calls", async () => {
  const s = sched();
  let calls = 0;
  const stop = poll(async () => { calls++; }, { onResult() {}, onError() {}, schedule: s.schedule });
  await flush();
  stop();
  expect(s.q.length).toBe(0);
  expect(calls).toBe(1);
});
