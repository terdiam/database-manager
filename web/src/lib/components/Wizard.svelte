<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.svelte.ts';

	let {
		steps,
		step = $bindable(0),
		finishLabel,
		canFinish = true,
		onfinish,
		children
	}: { steps: string[]; step?: number; finishLabel: string; canFinish?: boolean; onfinish: () => void; children: Snippet } = $props();

	// A form per step: the browser validates required fields before moving on.
	function submit(event: SubmitEvent) {
		event.preventDefault();
		if (step < steps.length - 1) step += 1;
		else onfinish();
	}
</script>

<ol class="mb-4 flex flex-wrap gap-2 text-sm">
	{#each steps as name, i}
		<li aria-current={i === step ? 'step' : undefined} class="flex items-center gap-2 rounded-full border px-3 py-1 {i === step ? 'border-brand font-semibold text-brand' : 'border-line text-dim'}">
			<span class="tabular-nums">{i < step ? '✓' : i + 1}</span>{name}
		</li>
	{/each}
</ol>

<form class="card" onsubmit={submit}>
	{@render children()}
	<div class="mt-5 flex justify-between border-t border-line pt-4">
		<button type="button" class="btn" disabled={step === 0} onclick={() => (step -= 1)}>{t('action.back')}</button>
		<button type="submit" class="btn btn-primary" disabled={step === steps.length - 1 && !canFinish}>
			{step === steps.length - 1 ? finishLabel : t('action.next')}
		</button>
	</div>
</form>
