import { formatClock } from "../time";

/** The live region always exists so screen readers announce changes inside it. */
export function StatusBanner(p: { failing: boolean; failingForMs: number; hintAfterMs: number; lastLoaded: number | null; finalizedThrough: number | null }) {
  return (
    <div role="status" aria-live="polite" class={p.failing ? "banner" : "muted"}>
      {p.failing ? (
        <>
          Tally is not running, retrying.
          {p.lastLoaded !== null && ` Showing what was last loaded (as of ${formatClock(p.lastLoaded)}).`}
          {p.failingForMs >= p.hintAfterMs && " If you restarted Tally, reopen the address it printed."}
        </>
      ) : p.finalizedThrough !== null ? (
        `Up to date through ${formatClock(p.finalizedThrough)}. Newest minutes appear about 2 minutes after they happen.`
      ) : null}
    </div>
  );
}
