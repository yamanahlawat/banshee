<script lang="ts">
  export let host: string;
  /// Set in from the page's gutter, where it stands above page content.
  export let inset = false;
</script>

<p class="host" class:inset>Audio goes to <span class="mono">{host}</span></p>

<style>
  .host {
    margin: 0;
    color: var(--dim);
    font-size: 11px;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  .inset {
    margin: 0 var(--gutter) 16px;
  }
</style>
