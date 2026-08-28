<script lang="ts">
  export let label: string;
  export let press: () => void;
</script>
<button
  type="button"
  onclick={press}
  style="font: inherit; font-size: 13px; font-weight: 600; color: var(--ink); background: transparent; border: 1.5px solid var(--ink); border-radius: 6px; padding: 4px 10px; min-height: 28px; cursor: pointer;"
>{label}</button>
