import { describe, expect, test } from "bun:test";
import { docsPage } from "../src/pages.ts";

const html = async () => await docsPage().text();

describe("docsPage", () => {
  test("renders 200 with docs title", async () => {
    expect(docsPage().status).toBe(200);
    expect(await html()).toContain("Docs");
  });

  test("has a per-harness setup section with each supported harness", async () => {
    const h = await html();
    expect(h).toContain("Set up your harness");
    for (const heading of [
      "Claude Code",
      "Codex, Aider, Gemini CLI",
      "Cursor, Windsurf, VSCodium, Trae, Antigravity",
      "VS Code",
      "JetBrains and other WakaTime-compatible editors",
      "GitHub Copilot",
    ])
      expect(h).toContain(`<h3>${heading}</h3>`);
  });

  test("mentions the exact CLI spellings", async () => {
    const h = await html();
    for (const cmd of [
      "tally setup",
      "tally doctor",
      "tally heartbeat-config",
      "tally@tally",
      "sectersion.tally",
      ".wakatime.cfg",
      "api_url",
      "api_key",
    ])
      expect(h).toContain(cmd);
  });
});
