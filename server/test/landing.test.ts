import { describe, test, expect } from "bun:test";
import { landingPage } from "../src/pages.ts";
import { CSP } from "../src/html.ts";

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
  test("CSP adds font-src only on landing", () => {
    const res = landingPage();
    expect(res.headers.get("content-security-policy")).toBe(CSP + " font-src 'self'");
    expect(res.headers.get("content-type")).toContain("text/html");
  });
});
