<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Illustration from './Illustration.svelte';
	let { title = t('empty.title'), hint = t('empty.hint'), action }: { title?: string; hint?: string; action?: Snippet } = $props();
</script>

<div class="card flex flex-col items-center gap-2 py-10 text-center">
	<Illustration name="empty" class="w-40" />
	<p class="font-medium">{title}</p>
	<p class="text-sm text-dim">{hint}</p>
	{@render action?.()}
</div>
