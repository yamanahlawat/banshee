import { getContext, setContext } from 'svelte';
import { readable, writable, type Readable } from 'svelte/store';

/// An `aria-describedby` list from the ids that apply. None leaves the
/// attribute off, since an empty list still names the attribute.
export function idList(...ids: (string | false | null | undefined)[]): string | undefined {
  return ids.filter(Boolean).join(' ') || undefined;
}

const ROW = Symbol('row description');

/// Called by a row: what it sets here describes every control inside it.
export function describeControls() {
  const ids = writable<string | undefined>(undefined);
  setContext<Readable<string | undefined>>(ROW, ids);
  return ids;
}

/// What the row around a control describes it by. Outside a row, nothing.
export function rowDescription(): Readable<string | undefined> {
  return getContext<Readable<string | undefined>>(ROW) ?? readable(undefined);
}
