import { render } from '@testing-library/svelte';
import { beforeEach, expect, it } from 'vitest';
import { daemon, downloadLine, empty, type Progress } from '../lib/daemon';
import { forgetSamples, record } from '../lib/downloads';
import DownloadLine from './DownloadLine.svelte';

const at = (bytes: number, state: Progress['state'] = 'downloading'): Progress => ({
  label: 'Voice',
  model: 'kokoro.onnx',
  index: 1,
  count: 2,
  bytes,
  total: 100,
  state,
});

beforeEach(() => {
  forgetSamples();
  record(at(10), 0);
  record(at(11), 5);
  record(at(20), 5005);
});

it('shows the pace while the file arrives', () => {
  daemon.set({ ...empty(), download: at(20) });
  const { container } = render(DownloadLine);
  expect(container.querySelector('.pace')?.textContent).toContain('B/s');
});

it('sets the pace apart from the size by a spaced dot', () => {
  daemon.set({ ...empty(), download: at(20) });
  const { container } = render(DownloadLine);
  const pace = container.querySelector('.pace')?.textContent;
  expect(container.querySelector('.progress')?.textContent?.trim()).toBe(
    `${downloadLine(at(20))} · ${pace}`,
  );
});

it('shows no pace once the file failed', () => {
  daemon.set({ ...empty(), download: at(20, 'failed') });
  const { container } = render(DownloadLine);
  expect(container.querySelector('.progress')?.textContent).toContain('failed');
  expect(container.querySelector('.pace')).toBeNull();
  expect(container.textContent).not.toContain('B/s');
});
