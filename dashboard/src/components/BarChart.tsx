import { formatDuration } from "../time";

/** Grouped bars per day: verified (solid) and unverified (hatched). Never summed. The caller renders the data table. */
export function BarChart(p: { id: string; title: string; labels: string[]; verified: number[]; unverified: number[] }) {
  const n = p.labels.length, W = 900, H = 240, L = 64, B = 28, T = 20;
  const max = Math.max(1, ...p.verified, ...p.unverified);
  const slot = Math.floor((W - L) / Math.max(n, 1)), bw = Math.max(2, Math.floor(slot * 0.4));
  const y = (v: number) => H - B - Math.floor((v * (H - B - T)) / max);
  const step = Math.ceil(n / 6);
  // a value label on every non-zero bar while there is room, else on the tallest bar of each series
  const show = (v: number, all: number[]) => v > 0 && (n <= 14 || v === Math.max(...all));
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-labelledby={`${p.id}-t`} aria-describedby={`${p.id}-table`}>
      <title id={`${p.id}-t`}>{p.title}</title>
      <defs>
        <pattern id="hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <rect width="5" height="5" fill="#f5e9d4" />
          <line x1="0" y1="0" x2="0" y2="5" stroke="#aa2d00" stroke-width="2" />
        </pattern>
      </defs>
      {[0, 1, 2].map((i) => (
        <g>
          <line class="grid-line" x1={L} y1={y((max * i) / 2)} x2={W} y2={y((max * i) / 2)} />
          <text x={L - 8} y={y((max * i) / 2) + 4} text-anchor="end">{formatDuration((max * i) / 2)}</text>
        </g>
      ))}
      {p.labels.map((l, i) => (
        <g>
          <rect class="v" x={L + i * slot} y={y(p.verified[i])} width={bw} height={H - B - y(p.verified[i])} />
          <rect class="u" x={L + i * slot + bw + 2} y={y(p.unverified[i])} width={bw} height={H - B - y(p.unverified[i])} />
          {show(p.verified[i], p.verified) && <text class="val" x={L + i * slot + bw} y={y(p.verified[i]) - 4} text-anchor="end">{formatDuration(p.verified[i])}</text>}
          {show(p.unverified[i], p.unverified) && <text class="val" x={L + i * slot + bw + 2} y={y(p.unverified[i]) - 4} text-anchor="start">{formatDuration(p.unverified[i])}</text>}
          {i % step === 0 && <text x={L + i * slot} y={H - 8}>{l}</text>}
        </g>
      ))}
    </svg>
  );
}
