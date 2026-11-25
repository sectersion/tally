import { formatDuration } from "../time";

// Stroke sizes in seconds; the first one that keeps the drawing under MAX_STROKES wins.
const UNITS = [60, 300, 600, 1800, 3600, 4 * 3600, 8 * 3600, 24 * 3600];
const MAX_STROKES = 100;

export function pickUnit(seconds: number): number {
  return UNITS.find((u) => seconds / u <= MAX_STROKES) ?? UNITS[UNITS.length - 1];
}

function Gate({ n, tone, dashed }: { n: number; tone: string; dashed: boolean }) {
  // Four upright strokes and, when the gate is full, a fifth across them: the classic tally gate.
  const w = 14 * 5 + 8;
  return (
    <svg class="gate" viewBox={`0 0 ${w} 64`} width={w} height="64" aria-hidden="true" focusable="false">
      <g stroke={tone} stroke-width="4" stroke-linecap="round" stroke-dasharray={dashed ? "5 4" : undefined}>
        {Array.from({ length: Math.min(n, 4) }, (_, i) => <line x1={8 + i * 14} y1="6" x2={8 + i * 14} y2="58" />)}
        {n === 5 && <line x1="2" y1="52" x2={w - 2} y2="12" stroke-dasharray={dashed ? "5 4" : undefined} />}
      </g>
    </svg>
  );
}

/** Each stroke is one fixed unit of time. Only whole strokes are drawn; the remainder is stated in words, never rounded up. */
/** `unit` should be shared by rows that are compared, so stroke counts stay comparable. */
export function Gates({ seconds, unit, tone, dashed = false, label }: { seconds: number; unit: number; tone: string; dashed?: boolean; label: string }) {
  const strokes = Math.floor(seconds / unit);
  const left = seconds - strokes * unit;
  const gates = Array.from({ length: Math.ceil(strokes / 5) }, (_, g) => Math.min(5, strokes - g * 5));
  return (
    <figure class="gates">
      <div class="gate-row" role="img" aria-label={`${label}: ${strokes} strokes of ${formatDuration(unit)} each`}>
        {gates.map((n) => <Gate n={n} tone={tone} dashed={dashed} />)}
      </div>
      <figcaption>
        {strokes === 0 ? `Less than one stroke yet (a stroke is ${formatDuration(unit)}).` : `Each stroke is ${formatDuration(unit)}.`}
        {strokes > 0 && left > 0 && ` ${formatDuration(left)} more is not drawn.`}
      </figcaption>
    </figure>
  );
}

/** Verified seconds per day as plain bars on a zero baseline (one scale, labelled with its maximum). */
export function Spark({ values, tone, label }: { values: number[]; tone: string; label: string }) {
  const max = Math.max(1, ...values), n = Math.max(values.length, 1), W = 560, H = 120, pitch = W / n;
  return (
    <figure class="spark">
      <svg viewBox={`0 0 ${W} ${H + 4}`} width="100%" role="img" aria-label={`${label}, tallest day ${formatDuration(max)}`} focusable="false">
        {values.map((v, i) => <rect x={i * pitch + 1} y={H - Math.round((v / max) * H)} width={Math.max(1, pitch - 3)} height={Math.round((v / max) * H)} fill={tone} />)}
        <line x1="0" y1={H + 1} x2={W} y2={H + 1} stroke={tone} stroke-width="2" />
      </svg>
      <figcaption>{label}. Tallest day {formatDuration(max)}.</figcaption>
    </figure>
  );
}
