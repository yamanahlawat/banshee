<script lang="ts">
  import { checklist, daemon, needleAt, stateWord } from '../lib/daemon';

  export let compact = false;

  const WIDTH = 480 - 44;

  $: rows = checklist($daemon);
  $: needle = needleAt(rows);
  $: blocked = rows.map((row, i) => (row.state === 'blocked' ? i : -1)).filter((i) => i >= 0);
  // Ink, not brass, while recording: the live colour belongs to the lamp and
  // the meter alone.
  $: needleColour = stateWord($daemon) === 'Recording' ? 'var(--ink)' : 'var(--live)';
  $: step = (WIDTH - 12) / (rows.length - 1);
  $: x = (i: number) => 6 + i * step;
  // The lower row repeats the even labels for the eye only, so a reader hears
  // each station once.
  function shownIn(row: number, i: number): boolean {
    return i % 2 === row;
  }
  $: justify = (i: number) => (i === 0 ? 'start' : i === rows.length - 1 ? 'end' : 'center');
  $: labelColour = (i: number) => (i === needle ? 'var(--ink)' : 'var(--dim)');
  $: columns = `${step / 2}px repeat(${rows.length - 2}, ${step}px) ${step / 2}px`;
</script>

<section
  aria-label={compact ? `Setup progress, ${rows[needle].station}` : 'Setup progress'}
  style="padding: {compact ? '6px' : '12px'} 22px 0; border-bottom: 1px solid var(--rule);"
>
  {#if !compact}
    <ol style="display: grid; grid-template-columns: {columns}; margin: 0 0 6px 0; padding: 0; min-height: 14px; list-style: none;">
      {#each rows as row, i}
        <li
          class="caps"
          aria-current={i === needle ? 'step' : undefined}
          style="margin: 0; padding: 0; justify-self: {justify(i)}; letter-spacing: 0.05em; color: {labelColour(i)};"
        >
          {#if shownIn(0, i)}{row.station}{:else}<span class="sr">{row.station}</span>{/if}<span class="sr">, {row.state}</span>
        </li>
      {/each}
    </ol>
  {/if}

  <svg width={WIDTH} height="30" viewBox="0 0 {WIDTH} 30" aria-hidden="true" style="display: block; overflow: visible;">
    <defs>
      <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--live)" stroke-width="2.5" />
      </pattern>
    </defs>
    <line x1="0" y1="14" x2={WIDTH} y2="14" stroke="var(--ink)" stroke-width="1.5" />
    {#each rows as _row, i}
      <line x1={x(i)} y1="8" x2={x(i)} y2="20" stroke="var(--ink)" stroke-width="1.5" />
    {/each}
    {#each blocked as i}
      <rect x={x(i) - 22} y="10" width="44" height="8" fill="url(#hatch)" />
    {/each}
    <g transform="translate({x(needle)} 0)">
      <line x1="0" y1="0" x2="0" y2="28" stroke={needleColour} stroke-width="2.5" />
      <path d="M-5 0 L5 0 L0 6 Z" fill={needleColour} />
    </g>
  </svg>

  {#if compact}
    <div style="height: 6px;"></div>
  {:else}
    <div aria-hidden="true" style="display: grid; grid-template-columns: {columns}; margin: 2px 0 10px 0; padding: 0; min-height: 14px;">
      {#each rows as row, i}
        <span
          class="caps"
          style="justify-self: {justify(i)}; letter-spacing: 0.05em; color: {labelColour(i)};"
        >{shownIn(1, i) ? row.station : ''}</span>
      {/each}
    </div>
  {/if}
</section>
