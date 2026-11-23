// Live project numbers: when the server reports a new change_seq, reload <main> from /fragment.
// The page works without this script. The browser reconnects by itself and sends Last-Event-ID,
// which the server prefers over the page-load ?seq=.
const el = document.querySelector("[data-project]");
if (el && "EventSource" in window) {
  const base = location.pathname.replace(/\/+$/, "");
  const es = new EventSource(`${base}/events?seq=${encodeURIComponent(el.dataset.seq ?? "")}`);
  let timer;
  es.addEventListener("changed", (e) => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        const r = await fetch(`${base}/fragment`, { cache: "no-store" });
        if (!r.ok) return;
        // Same-origin HTML that the server already escaped; it carries the new data-seq.
        document.querySelector("main").innerHTML = await r.text();
        const now = document.querySelector("[data-project]");
        if (now && Number(e.data) > Number(now.dataset.seq)) now.dataset.seq = e.data;
      } catch {
        // offline: the next event retries
      }
    }, 500);
  });
}
