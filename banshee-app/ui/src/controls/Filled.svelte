<script lang="ts">
  export let label: string;
  export let press: () => void;
</script>
<button
  type="button"
  onclick={press}
  style="font: inherit; background: var(--ink); color: var(--face); border: 1.5px solid var(--ink); border-radius: 6px; padding: 5px 12px; font-size: 13px; font-weight: 600; min-height: 28px; cursor: pointer;"
>{label}</button>
