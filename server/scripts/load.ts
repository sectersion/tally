// Tiny load generator.
//   bun scripts/load.ts <url> [concurrency=32] [seconds=5] [etag]   requests/second
//   bun scripts/load.ts sse <url> <n> [hold_seconds=20]             hold n SSE streams open
// Each stream sends its own x-forwarded-for, so run it against scripts/loadserver.ts (TRUST_PROXY).
const [a, b, c, d] = process.argv.slice(2) as string[];
if (a === "sse") {
  const n = Number(c);
  let ok = 0;
  await Promise.all(
    Array.from({ length: n }, async (_, i) => {
      const r = await fetch(b as string, {
        headers: { "x-forwarded-for": `10.${i >> 16}.${(i >> 8) & 255}.${i & 255}` },
      });
      if (r.status !== 200) return console.error(`stream ${i}: ${r.status}`);
      ok++;
      const rd = r.body?.getReader();
      void (async () => {
        while (rd && !(await rd.read()).done);
      })();
    }),
  );
  console.log(`open streams: ${ok}/${n}`);
  await Bun.sleep(Number(d ?? 20) * 1000);
  process.exit(0);
}
const conc = Number(b ?? 32);
const secs = Number(c ?? 5);
const headers: Record<string, string> = d ? { "if-none-match": d } : {};
const stat: Record<number, number> = {};
const end = Date.now() + secs * 1000;
let n = 0;
await Promise.all(
  Array.from({ length: conc }, async () => {
    while (Date.now() < end) {
      const r = await fetch(a as string, { headers });
      await r.arrayBuffer();
      stat[r.status] = (stat[r.status] ?? 0) + 1;
      n++;
    }
  }),
);
console.log(`${a} conc=${conc} ${(n / secs).toFixed(0)} req/s statuses=${JSON.stringify(stat)}`);
