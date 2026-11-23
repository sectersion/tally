import { useState } from "preact/hooks";
import { dayEdges, dayLabel, formatDuration } from "../time";
import { BarChart } from "./BarChart";
import { DataTable } from "./DataTable";
import { Period, projectLabel, useLoad, ViewProps } from "./util";

export function TimeView(p: ViewProps) {
  const [days, setDays] = useState(28);
  const [project, setProject] = useState("");
  const [edges, setEdges] = useState<number[]>([]);
  const h = useLoad(async () => {
    const e = dayEdges(p.now(), days);
    const r = await p.api.hours(e);
    setEdges(e);
    return r;
  }, [p.api, days, p.tick]);
  const rows = (h.data?.data.days ?? []).filter((d) => project === "" || (d.project ?? "") === project);
  const n = Math.max(edges.length - 1, 0);
  const v = Array<number>(n).fill(0), u = Array<number>(n).fill(0);
  for (const r of rows) {
    if (r.day < n) { v[r.day] += r.seconds.verified_s; u[r.day] += r.seconds.unverified_s; }
  }
  const projectsSeen = [...new Set((h.data?.data.days ?? []).map((d) => d.project ?? ""))];
  const label = (d: number) => (edges[d] === undefined ? "" : dayLabel(edges[d]));
  return (
    <div>
      <h2 id="time-h" tabIndex={-1}>Hours per day</h2>
      <Period days={days} onChange={setDays} />{" "}
      <label>
        Project{" "}
        <select value={project} onChange={(e) => setProject((e.target as HTMLSelectElement).value)}>
          <option value="">All projects</option>
          {projectsSeen.map((k) => <option value={k}>{projectLabel(k || null, p.projects)}</option>)}
        </select>
      </label>
      {h.data ? (
        <>
          <p class="muted">
            Solid bars are verified time, hatched bars are unverified time. They are never added together.
            An action counts on the day it ended, even if its interval began before midnight.
          </p>
          <BarChart id="hours" title="Verified and unverified time per day" labels={Array.from({ length: n }, (_, i) => label(i))} verified={v} unverified={u} />
          <DataTable
            id="hours-table"
            caption="Time per day and project"
            head={["Day", "Project", "Verified", "Unverified", "Removed"]}
            rows={rows.map((r) => [label(r.day), projectLabel(r.project, p.projects), formatDuration(r.seconds.verified_s), formatDuration(r.seconds.unverified_s), formatDuration(r.seconds.removed_s)])}
          />
        </>
      ) : <p>Loading time</p>}
    </div>
  );
}
