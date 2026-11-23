// Live project numbers: when the server reports a new change_seq, reload <main> from /fragment.
// The page works without this script. Native reconnects send Last-Event-ID; a stream the browser
// gave up on (a 429 or a 503 during a deploy) is reopened after 30 s with the current data-seq.
const el = document.querySelector("[data-project]");
if (el && "EventSource" in window) {
  const base = location.pathname.replace(/\/+$/, "");
  const seq = () => document.querySelector("[data-project]")?.dataset.seq ?? "";
  let timer;
  let reopen;
  const refresh = async (n, retried) => {
    try {
      const r = await fetch(`${base}/fragment`, { cache: "no-store" });
      if (!r.ok) throw new Error(String(r.status));
      // Same-origin HTML that the server already escaped; it carries the new data-seq.
      document.querySelector("main").innerHTML = await r.text();
      const now = document.querySelector("[data-project]");
      if (now && Number(n) > Number(now.dataset.seq)) now.dataset.seq = n;
    } catch {
      if (!retried) timer = setTimeout(() => refresh(n, true), 5000);
    }
  };
  const open = () => {
    const es = new EventSource(`${base}/events?seq=${encodeURIComponent(seq())}`);
    es.addEventListener("changed", (e) => {
      clearTimeout(timer);
      timer = setTimeout(() => refresh(e.data, false), 500);
    });
    es.addEventListener("error", () => {
      if (es.readyState !== EventSource.CLOSED) return; // the browser is reconnecting itself
      clearTimeout(reopen);
      reopen = setTimeout(open, 30_000);
    });
  };
  open();
}
