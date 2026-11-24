// bun run e2e/gen-large.ts > large.ndjson   (2,000,000 events over 90 days, hashes only; for manual measurement)
const N = Number(process.env.N ?? 2_000_000);
const end = Date.now();
const start = end - 90 * 86_400_000;
const step = (end - start) / N;
const out: string[] = [];
const flush = () => { if (out.length) { process.stdout.write(out.join("\n") + "\n"); out.length = 0; } };
for (let i = 0; i < N; i++) {
  const ts = Math.floor(start + i * step);
  const base = { v: 1, id: `g${i}`, ts, source: "gen", project: i % 3 === 0 ? "p2" : "p1", session: null };
  const r = i % 100;
  const e =
    r === 0 ? { ...base, kind: "test_run", cmd_hash: `c${(i / 100) % 7}`, exit_code: (i / 100) % 3 === 0 ? 1 : 0, initiator: "user" }
    : r === 50 ? { ...base, kind: "prompt", chars: 200, relevance_pm: (i * 7) % 1000, novelty_bytes: 100 }
    : r % 10 === 5 ? { ...base, kind: "user_edit", file_hash: `f${i % 50}`, lines_changed: 3 }
    : { ...base, kind: "agent_output", chars: 500 };
  out.push(JSON.stringify(e));
  if (out.length >= 5000) flush();
}
flush();
