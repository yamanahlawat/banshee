<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import Job from '../bands/Job.svelte';
  import Row from '../controls/Row.svelte';
  import Toggle from '../controls/Toggle.svelte';

  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  $: daemonConfig = (config.daemon ?? {}) as Record<string, unknown>;
  $: saveHistory = daemonConfig.save_history !== false;
</script>

<Job name="History settings">
  <Row name="Save what I say" command={`banshee config set daemon.save_history ${saveHistory}`} pending={$daemon.pending.has('daemon.save_history')}>
    <Toggle on={saveHistory} label="Save history" change={(next) => write('daemon.save_history', next)} />
    <span>{saveHistory ? 'On' : 'Off'}</span>
  </Row>
</Job>
