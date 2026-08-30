/// The three speech models the daemon offers. The labels are the window's
/// words; what a choice costs to fetch is the daemon's, because only it knows
/// which files are already here.
export const PRESETS = [
  { value: 'fast', label: 'Fast' },
  { value: 'balanced', label: 'Balanced' },
  { value: 'quality', label: 'Quality' },
];

/// What the daemon says the pending run costs. Only the daemon knows which
/// files are already here, so a sum taken in the window states the cost of a
/// first run whatever is actually missing.
export function downloadSize(megabytes: number): string {
  return megabytes >= 1000 ? `${(megabytes / 1000).toFixed(1)} GB` : `${megabytes} MB`;
}
