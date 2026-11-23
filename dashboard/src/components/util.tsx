import { useEffect, useState } from "preact/hooks";
import type { ApiLike } from "../api";

export type ViewProps = {
  api: ApiLike;
  now: () => number;
  /** Changes when the daemon has new final data; views reload on it. */
  tick: number | null;
  projects: { key: string; label: string }[];
};

/** Loads on mount and when `deps` change; keeps the previous data while reloading or after a failure. */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[]): { data?: T; err?: unknown } {
  const [s, set] = useState<{ data?: T; err?: unknown }>({});
  useEffect(() => {
    let live = true;
    fn().then(
      (data) => live && set({ data }),
      (err) => live && set((p) => ({ ...p, err })),
    );
    return () => { live = false; };
  }, deps);
  return s;
}

/** A project's display name: its label, else the first 8 of a 64-hex id, else the text itself (rendered as text only). */
export function projectLabel(p: string | null, projects: { key: string; label: string }[]): string {
  if (p === null) return "no project";
  return projects.find((x) => x.key === p)?.label ?? (/^[0-9a-f]{64}$/.test(p) ? p.slice(0, 8) : p);
}

export function Period({ days, onChange }: { days: number; onChange: (d: number) => void }) {
  return (
    <label>
      Period{" "}
      <select value={days} onChange={(e) => onChange(Number((e.target as HTMLSelectElement).value))}>
        {[7, 28, 90].map((d) => <option value={d}>{`last ${d} days`}</option>)}
      </select>
    </label>
  );
}
