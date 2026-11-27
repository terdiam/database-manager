<script lang="ts">
	import type { Status } from '#lib/api/index.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon, { type IconName } from './Icon.svelte';

	let { status }: { status: Status } = $props();

	// Icon + text + color: color is never the only signal.
	const look: Record<Status, [IconName, string]> = {
		completed: ['check', 'text-ok'],
		online: ['check', 'text-ok'],
		running: ['spin', 'text-brand'],
		pending: ['clock', 'text-warn'],
		failed: ['alert', 'text-bad'],
		offline: ['alert', 'text-bad']
	};
</script>

<span class="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs font-medium {look[status][1]}">
	<Icon name={look[status][0]} size={13} class={status === 'running' ? 'motion-safe:animate-spin' : ''} />
	{t(`status.${status}`)}
</span>
