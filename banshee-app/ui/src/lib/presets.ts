/// The labels are the window's words; the cost is the daemon's.
export const PRESETS = [
  { value: 'fast', label: 'Fast' },
  { value: 'balanced', label: 'Balanced' },
  { value: 'quality', label: 'Quality' },
];

/// Only the daemon knows which files are already here, so the window never sums a cost itself.
export function downloadSize(megabytes: number): string {
  return megabytes >= 1000 ? `${(megabytes / 1000).toFixed(1)} GB` : `${megabytes} MB`;
}
