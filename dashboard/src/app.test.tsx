import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { ApiError } from "./api";
import { App } from "./components/App";
import { Onboarding } from "./components/Onboarding";
import { dayEdges } from "./time";
import { bugs, emptyStatus, env, fakeApi, NOW, noBugs, page, status } from "./fixtures";

afterEach(() => { cleanup(); location.hash = ""; Object.defineProperty(document, "hidden", { value: false, configurable: true }); });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const app = (api = fakeApi(), extra = {}) => render(<App api={api} pollMs={10} now={() => NOW} {...extra} />);

test("onboarding shows when the API says 503 and when there are no events", async () => {
  app(fakeApi({ status: async () => { throw new ApiError(503, "no database yet"); } }));
  await screen.findByText(/Get started/);
  cleanup();
  app(fakeApi({ status: async () => env(emptyStatus) }));
  await screen.findByText(/Get started/);
  expect(screen.getByText(/No source has reported yet/)).toBeTruthy();
});

test("onboarding lists the three steps in order and the never-recorded list", () => {
  render(<Onboarding status={null} />);
  const steps = [...document.querySelectorAll("ol > li")].map((l) => l.textContent);
  expect(steps[0]).toMatch(/tally run/);
  expect(steps[1]).toMatch(/tally project add/);
  expect(steps[2]).toMatch(/fills in within a few minutes/);
  expect(screen.getByText("What it never records")).toBeTruthy();
});

test("exactly one h1 and landmarks header, nav, main, footer exist", async () => {
  app();
  await screen.findByText(/Bugs fixed/);
  expect(document.querySelectorAll("h1").length).toBe(1);
  for (const t of ["header", "nav", "main", "footer"]) expect(document.querySelectorAll(t).length).toBe(1);
});

test("daemon restart: poll fails, banner appears with role=status, last data stays visible; next success removes the banner", async () => {
  let fail = false;
  app(fakeApi({ status: async () => { if (fail) throw new ApiError(0, "down"); return env(status); } }));
  await screen.findByText(/Bugs fixed/);
  fail = true;
  await waitFor(() => expect(screen.getByText(/Tally is not running, retrying/).closest('[role="status"]')).toBeTruthy());
  expect(screen.getByText(/Showing what was last loaded/)).toBeTruthy();
  expect(screen.getByText(/Bugs fixed/)).toBeTruthy();
  fail = false;
  await waitFor(() => expect(screen.queryByText(/not running/)).toBeNull(), { timeout: 2000 });
});

test("after five minutes of failures the banner adds the reopen-the-address hint", async () => {
  let t = NOW, fail = false;
  app(fakeApi({ status: async () => { if (fail) throw new ApiError(0, "down"); return env(status); } }), { now: () => t });
  await screen.findByText(/Bugs fixed/);
  fail = true;
  await screen.findByText(/not running/);
  expect(screen.queryByText(/reopen the address/)).toBeNull();
  t += 301_000;
  await screen.findByText(/reopen the address it printed/);
});

test("polling pauses while the tab is hidden and resumes when visible", async () => {
  const st = mock(async () => env(status));
  app(fakeApi({ status: st }));
  await screen.findByText(/Bugs fixed/);
  Object.defineProperty(document, "hidden", { value: true, configurable: true });
  await sleep(50);
  const n = st.mock.calls.length;
  await sleep(100);
  expect(st.mock.calls.length).toBe(n);
  Object.defineProperty(document, "hidden", { value: false, configurable: true });
  await sleep(100);
  expect(st.mock.calls.length).toBeGreaterThan(n);
});

test("data arriving while open: a second poll with more replay items appends them, keeps existing order and announces the count", async () => {
  let calls = 0;
  const more = { ...page, items: [...page.items, { ...page.items[0], ts: page.items[0].ts + 1_000_000, kind: "commit" as const, n: 12 }] };
  location.hash = "#/replay";
  let through = NOW - 120_000;
  const api = fakeApi({
    status: async () => env(status, through),
    replay: async () => env(++calls === 1 ? page : more),
  });
  app(api);
  await screen.findByText("bug fixed");
  const before = [...document.querySelectorAll("ol.timeline li")].map((l) => l.textContent);
  expect(before.length).toBe(3);
  through += 60_000; // new final data
  await screen.findByText("commit");
  const after = [...document.querySelectorAll("ol.timeline li")].map((l) => l.textContent);
  expect(after.slice(0, 3)).toEqual(before);
  expect(screen.getByText("1 new moments")).toBeTruthy();
});

test("changing the period requests new edges and from/to for the local calendar", async () => {
  const b = mock(async (_f: number, _t: number) => env(bugs));
  const h = mock(async (e: number[]) => env({ ...(await fakeApi().hours(e)).data }));
  app(fakeApi({ bugs: b, hours: h }));
  await screen.findByText(/Bugs fixed/);
  fireEvent.change(screen.getByLabelText(/Period/), { target: { value: "7" } });
  const e = dayEdges(NOW, 7);
  await waitFor(() => expect(b.mock.calls.some((c) => c[0] === e[0] && c[1] === e[7])).toBe(true));
  expect(h.mock.calls.some((c) => (c[0] as number[]).length === 8)).toBe(true);
});

test("project names are rendered as text, not HTML", async () => {
  const evil = "<img src=x onerror=alert(1)>";
  const b = { ...noBugs, fixed: 1, avg_ms_to_fix: 1000, fixes: [{ ...bugs.fixes[0], project: evil }] };
  app(fakeApi({ bugs: async () => env(b) }));
  await screen.findByText(new RegExp(evil.replace(/[()]/g, "\\$&")));
  expect(document.querySelector("img")).toBeNull();
});
