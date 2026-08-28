<script lang="ts">
  import { daemon } from '../lib/daemon';
  import { write } from '../lib/settings';
  import Job from '../bands/Job.svelte';
  import Row from '../controls/Row.svelte';
  import More from '../controls/More.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import Segmented from '../controls/Segmented.svelte';

  const LEVELS = [
    { value: 'error', label: 'Quiet' },
    { value: 'info', label: 'Normal' },
    { value: 'debug', label: 'Everything' },
  ];

  $: config = ($daemon.status?.config ?? {}) as Record<string, Record<string, unknown>>;
  $: daemonConfig = (config.daemon ?? {}) as Record<string, unknown>;
  $: saveHistory = daemonConfig.save_history !== false;
  $: alwaysOn = daemonConfig.always_on === true;
  $: level = String((config.logging ?? {}).level ?? 'info');
</script>

<Job name="History settings">
  <Row name="Save what I say" command={`banshee config set daemon.save_history ${saveHistory}`} pending={$daemon.pending.has('daemon.save_history')}>
    <Toggle on={saveHistory} label="Save history" change={(next) => write('daemon.save_history', next)} />
    <span>{saveHistory ? 'On' : 'Off'}</span>
  </Row>

  <More />

  <Row name="Keep listening" command={`banshee config set daemon.always_on ${alwaysOn}`} pending={$daemon.pending.has('daemon.always_on')}>
    <Toggle on={alwaysOn} label="Always on" change={(next) => write('daemon.always_on', next)} />
    <span>{alwaysOn ? 'On' : 'Off. Banshee listens only while the key is held.'}</span>
  </Row>

  <Row name="Log detail" command={`banshee config set logging.level ${level}`} pending={$daemon.pending.has('logging.level')}>
    <Segmented label="Log detail" active={level} options={LEVELS} change={(next) => write('logging.level', next)} />
  </Row>
</Job>
