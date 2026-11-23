import { afterAll, expect, test } from "bun:test";
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createLive } from "../src/live.ts";
import { bumpChange } from "../src/projects.ts";
import { chains, seed } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const SECRET = "live-secret";
const GENESIS = "0".repeat(64);
const servers: { stop: (force: boolean) => unknown }[] = [];
afterAll(async () => {
  for (const s of servers) s.stop(true);
  await sql.close();
});

function serve() {
  const live = createLive(sql);
  const s = createServer({
    cfg: testConfig({ githubWebhookSecret: SECRET }),
    sql,
    port: 0,
    live,
  });
  servers.push(s);
  return { url: `http://localhost:${s.port}`, live };
}

const rnd = () => Math.floor(Math.random() * 1e12);
async function owner() {
  const id = crypto.randomUUID();
  const handle = `u${id.replaceAll("-", "").slice(0, 12)}`;
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  return { id, handle };
}
/** A public, owned project; returns its id and public name. */
async function project(o: { public?: boolean } = {}) {
  const u = await owner();
  const id = crypto.randomUUID();
  const name = `${u.handle}/p${rnd()}`;
  await sql`insert into projects (id, owner_id, public_name, public) values (${id}, ${u.id}, ${name}, ${o.public ?? true})`;
  return { id, name };
}
const bump = (id: string) => sql.begin((tx) => bumpChange(tx, id));
const curSeq = async (id: string) =>
  Number((await sql`select change_seq from projects where id = ${id}`)[0].change_seq);

type Stream = {
  res: Response;
  next: () => Promise<string>;
  nextWithin: (ms: number) => Promise<string | null>;
  close: () => void;
};
async function openStream(url: string, headers: Record<string, string> = {}): Promise<Stream> {
  const ac = new AbortController();
  const res = await fetch(url, { headers, signal: ac.signal });
  const reader = res.body?.getReader();
  const dec = new TextDecoder();
  let buf = "";
  let pending: ReturnType<NonNullable<typeof reader>["read"]> | null = null;
  const nextWithin = async (ms: number): Promise<string | null> => {
    const deadline = Date.now() + ms;
    for (;;) {
      const i = buf.indexOf("\n\n");
      if (i >= 0) {
        const msg = buf.slice(0, i + 2);
        buf = buf.slice(i + 2);
        return msg;
      }
      if (!reader) return null;
      pending ??= reader.read();
      const left = deadline - Date.now();
      if (left <= 0) return null;
      const r = await Promise.race([pending, Bun.sleep(left).then(() => null)]);
      if (r === null) return null;
      pending = null;
      if (r.done) return "<eof>";
      buf += dec.decode(r.value, { stream: true });
    }
  };
  return {
    res,
    next: async () => (await nextWithin(3000)) ?? "<timeout>",
    nextWithin,
    close: () => ac.abort(),
  };
}
/** Polls until f() is true (server-side cleanup runs after the client's abort). */
async function until(f: () => boolean, ms = 2000) {
  const end = Date.now() + ms;
  while (!f() && Date.now() < end) await Bun.sleep(10);
  return f();
}
const upload = (url: string, p: { projectId: string; token: string }, entries: unknown[]) =>
  fetch(`${url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: { authorization: `Bearer ${p.token}`, "content-type": "application/json" },
    body: JSON.stringify({ prev_hash: GENESIS, entries }),
  });
async function fixtureProject(name: string) {
  const p = await seed(sql, name);
  const u = await owner();
  await sql`update projects set owner_id = ${u.id} where id = ${p.projectId}`;
  return p;
}

test("stream starts with retry and sends changed after an upload", async () => {
  const { url, live } = serve();
  const p = await fixtureProject("acme/demo-17");
  const s = await openStream(`${url}/p/acme/demo-17/events?seq=0`);
  expect(s.res.status).toBe(200);
  expect(s.res.headers.get("content-type")).toBe("text/event-stream");
  expect(s.res.headers.get("cache-control")).toBe("no-store");
  expect(s.res.headers.get("x-accel-buffering")).toBe("no");
  expect(await s.next()).toContain("retry: 3000");
  expect(await s.nextWithin(300)).toBeNull(); // seq 0 is current: nothing to send
  expect((await upload(url, p, (chains["acme/demo-17"] ?? []).slice(0, 3))).status).toBe(200);
  const msg = await s.nextWithin(3000);
  expect(msg).toMatch(/id: 1\nevent: changed\ndata: 1\n\n/);
  expect(msg).not.toMatch(/verified|unverified/);
  s.close();
  expect(await until(() => live.size().streams === 0 && live.size().projects === 0)).toBe(true);
});

test("a push receipt also triggers changed", async () => {
  const { url } = serve();
  const p = await project();
  const repo = 1_100_000 + (rnd() % 1_000_000);
  const inst = 5_100_000 + (rnd() % 1_000_000);
  await sql`insert into github_installations (id, account_login) values (${inst}, 'octo')`;
  await sql`insert into github_repos (id, full_name, installation_id) values (${repo}, ${`octo/live${repo}`}, ${inst})`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${p.id}, ${repo})`;
  const s = await openStream(`${url}/p/${p.name}/events?seq=0`);
  expect(await s.next()).toContain("retry: 3000");
  const f = JSON.parse(
    readFileSync(new URL("./fixtures/github/push.push.json", import.meta.url), "utf8"),
  ) as { headers: Record<string, string>; body: string };
  const payload = JSON.parse(f.body);
  payload.repository.id = repo;
  payload.installation.id = inst;
  const body = JSON.stringify(payload);
  const r = await fetch(`${url}/github/webhook`, {
    method: "POST",
    headers: {
      ...f.headers,
      "x-github-delivery": crypto.randomUUID(),
      "x-hub-signature-256": `sha256=${createHmac("sha256", SECRET).update(body).digest("hex")}`,
    },
    body,
  });
  expect(r.status).toBe(200);
  expect(await s.nextWithin(3000)).toMatch(/id: 1\nevent: changed\ndata: 1\n\n/);
  s.close();
});

test("reconnect with Last-Event-ID equal to current sends nothing; older sends changed at once", async () => {
  // Review Focus 5
  const { url } = serve();
  const p = await project();
  await bump(p.id);
  await bump(p.id);
  const cur = await curSeq(p.id);
  // Last-Event-ID wins over the page-load ?seq= on reconnects.
  const same = await openStream(`${url}/p/${p.name}/events?seq=0`, {
    "last-event-id": String(cur),
  });
  expect(await same.next()).toContain("retry: 3000");
  expect(await same.nextWithin(400)).toBeNull();
  const older = await openStream(`${url}/p/${p.name}/events?seq=${cur}`, {
    "last-event-id": String(cur - 1),
  });
  expect(await older.next()).toContain("retry: 3000");
  expect(await older.nextWithin(1000)).toBe(`id: ${cur}\nevent: changed\ndata: ${cur}\n\n`);
  // Both stay live.
  await bump(p.id);
  const want = `id: ${cur + 1}\nevent: changed\ndata: ${cur + 1}\n\n`;
  expect(await same.nextWithin(3000)).toBe(want);
  expect(await older.nextWithin(3000)).toBe(want);
  same.close();
  older.close();
});

test("a second server process delivers the change to the first's stream", async () => {
  const a = serve();
  const b = serve();
  const p = await fixtureProject("acme/demo-18");
  const s = await openStream(`${a.url}/p/acme/demo-18/events?seq=0`);
  expect(await s.next()).toContain("retry: 3000");
  expect((await upload(b.url, p, (chains["acme/demo-18"] ?? []).slice(0, 3))).status).toBe(200);
  expect(await s.nextWithin(3000)).toMatch(/id: 1\nevent: changed/);
  s.close();
});

test("closeAll ends open streams", async () => {
  const { url, live } = serve();
  const p = await project();
  const s1 = await openStream(`${url}/p/${p.name}/events`);
  const s2 = await openStream(`${url}/p/${p.name}/events`);
  expect(await s1.next()).toContain("retry: 3000");
  expect(await s2.next()).toContain("retry: 3000");
  live.closeAll();
  expect(live.size()).toEqual({ streams: 0, projects: 0, ips: 0 });
  // No Last-Event-ID or seq: the snapshot arrives first, then the end.
  for (const s of [s1, s2]) {
    let m = await s.next();
    while (m.startsWith("id:")) m = await s.next();
    expect(m).toBe("<eof>");
  }
  const late = await fetch(`${url}/p/${p.name}/events`); // shutting down: no new streams
  expect(late.status).toBe(503);
  await late.text();
});

test("fifth stream from one IP is 429; closed streams free their slot", async () => {
  const { url, live } = serve();
  const p = await project();
  const open: Stream[] = [];
  for (let i = 0; i < 4; i++) {
    const s = await openStream(`${url}/p/${p.name}/events`);
    expect(s.res.status).toBe(200);
    open.push(s);
  }
  const fifth = await fetch(`${url}/p/${p.name}/events`);
  expect(fifth.status).toBe(429);
  expect(fifth.headers.get("retry-after")).toBe("30");
  await fifth.text();
  for (const s of open) s.close();
  expect(await until(() => live.size().streams === 0)).toBe(true);
  expect(live.size()).toEqual({ streams: 0, projects: 0, ips: 0 });
  const again = await openStream(`${url}/p/${p.name}/events`);
  expect(again.res.status).toBe(200);
  again.close();
});

test("a client that leaves before the first write leaks nothing", async () => {
  const { url, live } = serve();
  const p = await project();
  for (let i = 0; i < 6; i++) {
    const ac = new AbortController();
    const r = fetch(`${url}/p/${p.name}/events`, { signal: ac.signal });
    ac.abort();
    await r.catch(() => null);
  }
  expect(await until(() => live.size().streams === 0)).toBe(true);
  expect(live.size()).toEqual({ streams: 0, projects: 0, ips: 0 });
});

test("events for a private project are 404", async () => {
  const { url, live } = serve();
  const p = await project({ public: false });
  const r = await fetch(`${url}/p/${p.name}/events`);
  expect(r.status).toBe(404);
  const missing = await fetch(`${url}/p/nobody${rnd()}/nope/events`);
  expect(await r.text()).toBe(await missing.text());
  const frag = await fetch(`${url}/p/${p.name}/fragment`);
  expect(frag.status).toBe(404);
  await frag.text();
  expect(live.size().streams).toBe(0);
});

/** Runs static/live.js against stubbed browser globals; timers fire only when the test says so. */
async function runLiveJs() {
  const src = await Bun.file(new URL("../static/live.js", import.meta.url)).text();
  const timers = new Map<number, { fn: () => unknown; ms: number }>();
  let nextId = 1;
  const sources: FakeES[] = [];
  class FakeES {
    static CLOSED = 2;
    readyState = 0;
    url: string;
    on = new Map<string, (e: { data?: string }) => void>();
    constructor(url: string) {
      this.url = url;
      sources.push(this);
    }
    addEventListener(t: string, f: (e: { data?: string }) => void) {
      this.on.set(t, f);
    }
  }
  const node = { dataset: { seq: "5" } };
  const main = { innerHTML: "" };
  const fetches: string[] = [];
  let fetchOk = false;
  const doc = {
    querySelector: (q: string) => (q === "main" ? main : q === "[data-project]" ? node : null),
  };
  const fakeFetch = async (u: string) => {
    fetches.push(u);
    if (!fetchOk) throw new TypeError("offline");
    return new Response(`<section id="numbers" data-project="x" data-seq="6"></section>`);
  };
  new Function(
    "document",
    "location",
    "window",
    "EventSource",
    "fetch",
    "setTimeout",
    "clearTimeout",
    src,
  )(
    doc,
    { pathname: "/p/a/b" },
    { EventSource: FakeES },
    FakeES,
    fakeFetch,
    (fn: () => unknown, ms: number) => {
      timers.set(nextId, { fn, ms });
      return nextId++;
    },
    (id: number) => timers.delete(id),
  );
  /** Fires the one pending timer with this delay, then lets its promises settle. */
  const fire = async (ms: number) => {
    const hit = [...timers].find(([, t]) => t.ms === ms);
    if (!hit) throw new Error(`no ${ms} ms timer`);
    timers.delete(hit[0]);
    await hit[1].fn();
    await Bun.sleep(0);
  };
  return {
    src,
    sources,
    timers,
    fire,
    fetches,
    main,
    node,
    setFetchOk: (v: boolean) => {
      fetchOk = v;
    },
  };
}

test("live.js reopens a stream the browser closed, and retries a failed fragment once", async () => {
  const b = await runLiveJs();
  expect(b.src.split("\n").length).toBeLessThan(40);
  expect(b.sources.map((s) => s.url)).toEqual(["/p/a/b/events?seq=5"]);
  const es = b.sources[0] as (typeof b.sources)[0];
  es.on.get("error")?.({}); // CONNECTING: the browser reconnects itself
  expect(b.timers.size).toBe(0);
  es.readyState = 2; // CLOSED: a 429 or 503 ends EventSource for good
  es.on.get("error")?.({});
  expect([...b.timers.values()].map((t) => t.ms)).toEqual([30_000]);
  b.node.dataset.seq = "7";
  await b.fire(30_000);
  expect(b.sources.map((s) => s.url)).toEqual(["/p/a/b/events?seq=5", "/p/a/b/events?seq=7"]);

  const es2 = b.sources[1] as (typeof b.sources)[0];
  es2.on.get("changed")?.({ data: "8" });
  await b.fire(500);
  expect(b.fetches).toEqual(["/p/a/b/fragment"]);
  expect([...b.timers.values()].map((t) => t.ms)).toEqual([5000]); // failed fetch: one retry
  await b.fire(5000);
  expect(b.fetches.length).toBe(2);
  expect(b.timers.size).toBe(0); // the retry failed too: no endless loop
  es2.on.get("changed")?.({ data: "9" });
  b.setFetchOk(true);
  await b.fire(500);
  expect(b.main.innerHTML).toContain('data-seq="6"');
  expect(b.timers.size).toBe(0);
});

test("fragment escapes like the page", async () => {
  const { url } = serve();
  const p = await project();
  const repo = rnd();
  await sql`insert into github_repos (id, full_name) values (${repo}, ${`o/<b>x${repo}`})`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${p.id}, ${repo})`;
  const frag = await fetch(`${url}/p/${p.name}/fragment`);
  expect(frag.status).toBe(200);
  expect(frag.headers.get("cache-control")).toBe("no-store");
  expect(frag.headers.get("content-type")).toContain("text/html");
  expect(frag.headers.get("referrer-policy")).toBe("same-origin");
  const f = await frag.text();
  expect(f).toContain(`&lt;b&gt;x${repo}`);
  expect(f).not.toContain("<b>x");
  expect(f).not.toContain("<main>");
  const pageHtml = await (await fetch(`${url}/p/${p.name}`)).text();
  expect(pageHtml).toContain(`<main>${f}</main>`);
});
