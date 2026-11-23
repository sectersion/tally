// Regenerates server/test/fixtures/github/*.json. Hand-built from GitHub's documented payload
// shapes (invented values), signed with the test secret. Run from server/: bun scripts/sign-fixture.ts
import { createHmac } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";

const SECRET = "test-secret";
const repo = { id: 1001, full_name: "octo/scratch", name: "scratch", owner: { login: "octo" } };
const inst = { id: 5001, account: { login: "octo" } };
const sender = { login: "octo", id: 42 };
const sha = (c: string) => c.repeat(40);
const author = { name: "A", email: "a@example.com" };

const fixtures: [string, string, string, object][] = [
  [
    "push",
    "push",
    "00000000-0000-4000-8000-000000000001",
    {
      ref: "refs/heads/main",
      before: sha("a"),
      after: sha("b"),
      forced: false,
      deleted: false,
      created: false,
      repository: repo,
      installation: { id: inst.id },
      sender,
      commits: [
        { id: sha("c"), timestamp: "2026-11-23T10:00:00Z", message: "invented first", author },
        { id: sha("b"), timestamp: "2026-11-23T10:05:00Z", message: "invented second", author },
      ],
      head_commit: {
        id: sha("b"),
        timestamp: "2026-11-23T10:05:00Z",
        message: "invented second",
        author,
      },
    },
  ],
  [
    "installation",
    "deleted",
    "00000000-0000-4000-8000-000000000002",
    { action: "deleted", installation: inst, repositories: [repo], requester: null, sender },
  ],
  [
    "installation_repositories",
    "removed",
    "00000000-0000-4000-8000-000000000003",
    {
      action: "removed",
      installation: inst,
      repository_selection: "selected",
      repositories_added: [],
      repositories_removed: [repo],
      requester: null,
      sender,
    },
  ],
  [
    "repository",
    "renamed",
    "00000000-0000-4000-8000-000000000004",
    {
      action: "renamed",
      repository: { ...repo, full_name: "octo/renamed", name: "renamed" },
      changes: { repository: { name: { from: "scratch" } } },
      installation: { id: inst.id },
      sender,
    },
  ],
  [
    "repository",
    "transferred",
    "00000000-0000-4000-8000-000000000005",
    {
      action: "transferred",
      repository: { ...repo, full_name: "newowner/scratch", owner: { login: "newowner" } },
      changes: { owner: { from: { user: { login: "octo", id: 42 } } } },
      installation: { id: 5002 },
      sender,
    },
  ],
  [
    "github_app_authorization",
    "revoked",
    "00000000-0000-4000-8000-000000000006",
    { action: "revoked", sender },
  ],
];

mkdirSync("test/fixtures/github", { recursive: true });
for (const [event, action, delivery, payload] of fixtures) {
  const body = JSON.stringify(payload);
  const sig = `sha256=${createHmac("sha256", SECRET).update(body).digest("hex")}`;
  const headers = {
    "content-type": "application/json",
    "user-agent": "GitHub-Hookshot/invented",
    "x-github-event": event,
    "x-github-delivery": delivery,
    "x-hub-signature-256": sig,
  };
  writeFileSync(
    `test/fixtures/github/${event}.${action}.json`,
    `${JSON.stringify({ headers, body }, null, 2)}\n`,
  );
}
