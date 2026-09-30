export const slot3 = (n: number) => String(n).padStart(3, "0");
export const fmtTime = (ms: number) => new Date(ms).toLocaleString();
export function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}
