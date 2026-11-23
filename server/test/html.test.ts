import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { esc, formatHours, formatMinutes, html, notFound, page, Safe } from "../src/html.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const srv = createServer({ cfg: testConfig(), sql, port: 0 });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

test("html tag escapes interpolations", () => {
  expect(html`<p>${"<script>x</script>"}</p>`.__html).toBe("<p>&lt;script&gt;x&lt;/script&gt;</p>");
  expect(html`<a title="${`" onmouseover="x`}">`.__html).not.toContain('" onmouseover');
  expect(html`<a title="${"' onfocus='x"}">`.__html).not.toContain("' onfocus");
  expect(esc(`&<>"'`)).toBe("&amp;&lt;&gt;&quot;&#39;");
});

test("nested html is not double-escaped; arrays join", () => {
  expect(html`<div>${html`<b>${"&"}</b>`}</div>`.__html).toBe("<div><b>&amp;</b></div>");
  expect(html`${[html`<i>${"<"}</i>`, "<"]}`.__html).toBe("<i>&lt;</i>&lt;");
});

test("a __html object from user data is not Safe", () => {
  const forged = JSON.parse('{"__html":"<script>x</script>"}') as unknown as Safe;
  expect(html`<p>${forged as unknown as string}</p>`.__html).not.toContain("<script>");
  expect(new Safe("<b>").__html).toBe("<b>");
});

test("page sets CSP, nosniff, referrer policy and cache by viewer", async () => {
  const anon = page("t", html`<p>x</p>`);
  const csp = anon.headers.get("content-security-policy") ?? "";
  expect(csp).toBe(
    "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
  );
  expect(anon.headers.get("x-content-type-options")).toBe("nosniff");
  expect(anon.headers.get("referrer-policy")).toBe("same-origin");
  expect(anon.headers.get("cache-control")).toBe("public, max-age=60");
  expect(anon.headers.get("content-type")).toBe("text/html; charset=utf-8");
  const body = await anon.text();
  expect(body).toContain("Sign in");
  expect(body).not.toContain("live.js");
  const me = page("t", html``, { viewer: { handle: "<me>", csrf: "c1" }, live: "x" });
  expect(me.headers.get("cache-control")).toBe("no-store");
  const t = await me.text();
  expect(t).toContain("&lt;me&gt;");
  expect(t).toContain('name="csrf" value="c1"');
  expect(t).toContain('<script src="/static/live.js" defer></script>');
});

test("notFound is a 404 html page", async () => {
  const r = notFound();
  expect(r.status).toBe(404);
  expect(r.headers.get("content-security-policy")).toContain("default-src 'none'");
});

test("formatters", () => {
  expect(formatHours(12.3 * 3600 + 20)).toBe("12.3 h");
  expect(formatHours(359)).toBe("0.0 h");
  expect(formatMinutes(179)).toBe("2 min");
  expect(formatMinutes(59)).toBe("<1 min");
  expect(formatMinutes(1)).toBe("<1 min");
});

test("static serves only the allowlist", async () => {
  const css = await fetch(`${base}/static/style.css`);
  expect(css.status).toBe(200);
  expect(css.headers.get("cache-control")).toBe("public, max-age=3600");
  expect(css.headers.get("content-type")).toContain("text/css");
  await css.text();
  const js = await fetch(`${base}/static/live.js`);
  expect(js.status).toBe(200);
  expect(js.headers.get("content-type")).toContain("javascript");
  expect((await js.text()).split("\n").length).toBeLessThan(40);
  for (const p of [
    "/static/../package.json",
    "/static/package.json",
    "/static/%2e%2e/package.json",
    "/static/review.js",
    "/static/",
  ]) {
    const r = await fetch(`${base}${p}`);
    expect(r.status).toBe(404);
    await r.text();
  }
});

test("no HTML route emits inline style= or on*= attributes", async () => {
  const id = crypto.randomUUID();
  const handle = `u${id.replaceAll("-", "").slice(0, 12)}`;
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  const { setCookie } = await createSession(sql, testConfig(), id);
  const cookie = setCookie.split(";")[0] as string;
  const r = await createProject(sql, id, `${handle}/p`);
  if (!("id" in r)) throw new Error("setup");
  const dc = (await (
    await fetch(`${base}/v1/device/code`, { method: "POST", body: '{"device_name":"lap"}' })
  ).json()) as { user_code: string };
  for (const p of [
    `/p/${handle}/p/settings`,
    `/p/${handle}/p`,
    `/p/${handle}/p/fragment`,
    `/u/${handle}`,
    "/",
    "/settings",
    "/device",
    `/device?code=${dc.user_code}`,
    "/device?code=nope",
    "/static/nope",
    "/static/live.js",
  ]) {
    const res = await fetch(`${base}${p}`, { headers: { cookie } });
    const text = await res.text();
    expect(text).not.toMatch(/\sstyle\s*=/i);
    expect(text).not.toMatch(/\son[a-z]+\s*=/i);
    if ((res.headers.get("content-type") ?? "").includes("html"))
      expect(res.headers.get("content-security-policy")).toContain("default-src 'none'");
  }
});
