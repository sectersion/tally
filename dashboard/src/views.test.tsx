import { afterEach, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/preact";
import axe from "axe-core";
import { Overview } from "./components/Overview";
import { Replay } from "./components/Replay";
import { TimeView } from "./components/TimeView";
import { App } from "./components/App";
import { bugs, env, fakeApi, hours, NOW, noBugs, prompts, status } from "./fixtures";

afterEach(cleanup);
const props = (o = {}) => ({ api: fakeApi(o), now: () => NOW, tick: 1, projects: status.projects });

test("a project with no fail-pass loops says so, shows 0 fixed and no average, and never renders NaN or 0 min", async () => {
  render(<Overview {...props({ bugs: async () => env(noBugs) })} />);
  await screen.findByText(/No fixed bugs in this period yet/);
  const t = document.body.textContent!;
  expect(t).not.toMatch(/NaN|Infinity|\b0 min/);
  expect(document.querySelector(".big")!.textContent).toBe("0");
});

test("an agent fix is labelled and kept out of the fixed number", async () => {
  const b = { ...bugs, fixed: 2, agent_fixes: 1, fixes: [{ ...bugs.fixes[0], by_agent: true }] };
  render(<Overview {...props({ bugs: async () => env(b) })} />);
  await screen.findByText(/1 more fixed by the agent/);
  expect(document.querySelector(".big")!.textContent).toBe("2");
  expect(screen.getByText(/includes fixes the agent completed/)).toBeTruthy();
  expect(screen.getByText(/fixed by the agent\)/)).toBeTruthy();
});

test("average time to fix is formatted from milliseconds", async () => {
  render(<Overview {...props()} />);
  await screen.findByText(/Average time from failure to fix: 12 min/);
});

test("prompt comparison shows not-enough-yet below 5 prompts and prints the thresholds from the response", async () => {
  const p = { ...prompts, enough: false, after_failure: { ...prompts.after_failure, prompts: 2 } };
  render(<Overview {...props({ prompts: async () => env(p) })} />);
  await screen.findByText(/Not enough failing runs yet to compare \(5 needed, 2 so far\)/);
  expect(screen.getByText(/at least 30 %/)).toBeTruthy();
  expect(screen.getByText(/within 5 minutes/)).toBeTruthy();
});

test("removed time table shows each reason with minutes and the capped row separately", async () => {
  render(<Overview {...props()} />);
  await screen.findByText("while you were away from the keyboard");
  expect(screen.getByText("both")).toBeTruthy();
  expect(screen.getByText(/exceeded the cap for that kind of action and is unverified/)).toBeTruthy();
  expect(screen.getByText(/never counted at all \(2 times\)/)).toBeTruthy();
});

test("no presence coverage is its own row, not in verified or removed totals", async () => {
  const h = { ...hours, no_presence: { reason: "no_presence_coverage", claims: 2, seconds: 600 } };
  render(<Overview {...props({ hours: async () => env(h) })} />);
  await screen.findByText(/No presence coverage, not counted: 10 min/);
  expect(screen.getByText(/was not counted/).textContent).toContain(`${Math.floor(hours.totals.removed_s / 60)} min`);
  expect(screen.queryByText(/skipped \(no presence coverage\)/)).toBeNull();
  const b = { ...bugs, skipped_fixes: 1 };
  cleanup();
  render(<Overview {...props({ bugs: async () => env(b) })} />);
  await screen.findByText("1 skipped (no presence coverage)");
});

test("time view keeps verified and unverified as separate series and table columns", async () => {
  render(<TimeView {...props()} />);
  await screen.findByRole("table");
  const heads = [...document.querySelectorAll("th[scope=col]")].map((h) => h.textContent);
  expect(heads).toEqual(["Day", "Project", "Verified", "Unverified", "Removed"]);
  expect(document.querySelectorAll("rect.v").length).toBe(28);
  expect(document.querySelectorAll("rect.u").length).toBe(28);
});

test("every chart has a data table and an accessible name", async () => {
  render(<TimeView {...props()} />);
  await screen.findByRole("table");
  const svg = document.querySelector("svg")!;
  expect(svg.getAttribute("role")).toBe("img");
  expect(document.getElementById(svg.getAttribute("aria-labelledby")!)!.textContent).toBeTruthy();
  expect(document.getElementById(svg.getAttribute("aria-describedby")!)!.tagName).toBe("TABLE");
  expect(document.querySelector("caption")).toBeTruthy();
});

test("the whole app passes axe with no violations (rules: all except color-contrast, which happy-dom cannot compute)", async () => {
  document.documentElement.lang = "en";
  document.title = "Tally";
  for (const hash of ["#/", "#/time", "#/replay"]) {
    location.hash = hash;
    const { container, unmount } = render(<App api={fakeApi()} now={() => NOW} pollMs={50} />);
    await screen.findAllByText(/Up to date through/);
    await new Promise((r) => setTimeout(r, 50));
    const r = await axe.run(container, { rules: { "color-contrast": { enabled: false }, region: { enabled: false } } });
    expect(r.violations.map((v) => `${hash} ${v.id}: ${v.nodes[0].html}`)).toEqual([]);
    unmount();
  }
  const { container } = render(<App api={fakeApi({ status: async () => env({ ...status, events: 0, final_claims: 0 }) })} now={() => NOW} pollMs={50} />);
  await screen.findByText(/Get started/);
  const r = await axe.run(container, { rules: { "color-contrast": { enabled: false }, region: { enabled: false } } });
  expect(r.violations.map((v) => v.id)).toEqual([]);
});

test("replay lists sessions with the will-not-see panel and a Load more that follows next", async () => {
  const pg = { items: [], next: "1:a", truncated: false };
  render(<Replay {...props({ replay: async (_f: number, _t: number, o: { after?: string }) => env(o.after ? { items: [], next: null, truncated: false } : pg) })} />);
  await screen.findByText(/What you see here and what you do not/);
  const btn = await screen.findByText("Load more");
  btn.click();
  await new Promise((r) => setTimeout(r, 20));
  expect(screen.queryByText("Load more")).toBeNull();
});
