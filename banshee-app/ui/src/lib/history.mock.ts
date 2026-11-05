import type { HistoryRow } from './tauri';

export const dictated = (id: number, text: string, timestamp: string): HistoryRow => ({
  id,
  kind: 'dictation',
  question: null,
  text,
  outcome: 'transcribed',
  reason: null,
  timestamp,
});
