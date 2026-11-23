import { formatDuration } from "../time";

/** Grouped bars per day: verified (solid) and unverified (hatched). Never summed. The caller renders the data table. */
export function BarChart(p: { id: string; title: string; labels: string[]; verified: number[]; unverified: number[] }) {
  const n = p.labels.length, W = 640, H = 200, L = 56, B = 24, T = 8;
  const max = Math.max(1, ...p.verified, ...p.unverified);
  const slot = Math.floor((W - L) / Math.max(n, 1)), bw = Math.max(1, Math.floor(slot * 0.4));
  const y = (v: number) => H - B - Math.floor((v * (H - B - T)) / max);
  const step = Math.ceil(n / 7);
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-labelledby={`${p.id}-t`} aria-describedby={`${p.id}-table`}>
      <title id={`${p.id}-t`}>{p.title}</title>
      <defs>
        <pattern id="hatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <line x1="0" y1="0" x2="0" y2="4" stroke="currentColor" stroke-width="2" />
        </pattern>
      </defs>
      <line x1={L} y1={H - B} x2={W} y2={H - B} stroke="currentColor" />
      {[0, 1, 2].map((i) => (
        <text x={L - 4} y={y((max * i) / 2) + 4} text-anchor="end">{formatDuration((max * i) / 2)}</text>
      ))}
      {p.labels.map((l, i) => (
        <g>
          <rect class="v" x={L + i * slot} y={y(p.verified[i])} width={bw} height={H - B - y(p.verified[i])} />
          <rect class="u" x={L + i * slot + bw} y={y(p.unverified[i])} width={bw} height={H - B - y(p.unverified[i])} />
          {i % step === 0 && <text x={L + i * slot} y={H - 8}>{l}</text>}
        </g>
      ))}
    </svg>
  );
}
