import { formatDuration } from "../time";

const RAMP = ["#e6efe9", "#b9d6c4", "#6fa585", "#2f6b46", "#0a2e0e"]; // tints of the forest block colour
const CW = 150, CH = 76, GAP = 8, TOP = 28;
const DOW = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** A calendar: one box per day, a row per week. Shade and the printed figure are verified time only; a coral corner marks days that also hold unverified time. */
export function Heatmap(p: { id: string; title: string; edges: number[]; verified: number[]; unverified: number[]; labels: string[] }) {
  const n = p.verified.length;
  const offset = (new Date(p.edges[0] ?? 0).getDay() + 6) % 7; // Monday first
  const rows = Math.ceil((n + offset) / 7);
  const max = Math.max(1, ...p.verified);
  const level = (v: number) => (v <= 0 ? -1 : Math.min(4, Math.floor((v / max) * 4.999)));
  const W = 7 * (CW + GAP) - GAP, H = TOP + rows * (CH + GAP);
  return (
    <svg class="heatmap" viewBox={`0 0 ${W} ${H}`} width={W} height={H} role="img" aria-labelledby={`${p.id}-t`} aria-describedby="hours-table">
      <title id={`${p.id}-t`}>{p.title}</title>
      {DOW.map((d, c) => <text class="dow" x={c * (CW + GAP) + 2} y={16}>{d}</text>)}
      {Array.from({ length: n }, (_, i) => {
        const idx = i + offset, c = idx % 7, r = Math.floor(idx / 7);
        const x = c * (CW + GAP), y = TOP + r * (CH + GAP), l = level(p.verified[i]);
        const d = new Date(p.edges[i]);
        const day = d.getDate() === 1 || i === 0 ? d.toLocaleDateString(undefined, { month: "short", day: "numeric" }) : String(d.getDate());
        const dark = l >= 3;
        return (
          <g>
            <rect x={x} y={y} width={CW} height={CH} rx="8" fill={l < 0 ? "#ffffff" : RAMP[l]} stroke={l < 0 ? "#9297a0" : "none"} stroke-dasharray={l < 0 ? "3 4" : undefined} />
            {p.unverified[i] > 0 && <path d={`M${x + CW - 22} ${y} h14 a8 8 0 0 1 8 8 v14 z`} fill="#aa2d00" />}
            <text class={`dn${dark ? " on-dark" : ""}`} x={x + 10} y={y + 22}>{day}</text>
            {l >= 0 && <text class={`dv${dark ? " on-dark" : ""}`} x={x + 10} y={y + CH - 12}>{formatDuration(p.verified[i])}</text>}
            <title>{`${p.labels[i]}: verified ${formatDuration(p.verified[i])}, unverified ${formatDuration(p.unverified[i])}`}</title>
          </g>
        );
      })}
    </svg>
  );
}

export function HeatmapLegend() {
  return (
    <p class="legend heat-legend" aria-hidden="true">
      Less verified
      {RAMP.map((c) => <span class="swatch" style={{ background: c }} />)}
      more
      <span class="corner" /> day also has unverified time
      <span class="swatch dashed" /> no verified time
    </p>
  );
}
