import { describe, expect, test } from "bun:test";
import { CSP } from "../src/html.ts";
import { landingPage } from "../src/pages.ts";

describe("landingPage", () => {
  test("structure and no script/inline style/csrf", async () => {
    const res = landingPage();
    const body = await res.text();
    expect((body.match(/<h1/g) ?? []).length).toBe(1);
    expect((body.match(/href="\/auth\/login"/g) ?? []).length).toBe(1);
    expect(body).toContain("View source");
    expect(body).toContain("github.com/Sectersion/tally");
    expect(body).not.toContain("<script");
    expect(body).not.toContain("csrf");
    expect(body).not.toContain("style=");
  });
  test("console mockup is aria-hidden", async () => {
    const body = await (await landingPage()).text();
    expect(body).toContain('<div class="console" aria-hidden="true">');
  });
  test("CSP self-hosts fonts on all pages; landing equals base CSP", () => {
    const res = landingPage();
    expect(res.headers.get("content-security-policy")).toBe(CSP);
    expect(CSP).toContain("font-src 'self'");
    expect(res.headers.get("content-type")).toContain("text/html");
  });
});
