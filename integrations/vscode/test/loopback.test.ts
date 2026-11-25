import { expect, test } from "bun:test";
import { isLoopbackUrl } from "../src/payload";

test("loopback http urls are accepted", () => {
  for (const u of ["http://127.0.0.1:25930/api", "http://localhost:1/api", "http://[::1]:1/api"]) expect(isLoopbackUrl(u)).toBe(true);
});

test("everything else is rejected", () => {
  for (const u of ["https://127.0.0.1:1/api", "http://example.com", "http://127.0.0.1.evil.com", "http://localhost.evil.com", "http://user@evil.com", "http://127.0.0.1@evil.com", "not a url", ""])
    expect(isLoopbackUrl(u)).toBe(false);
});
