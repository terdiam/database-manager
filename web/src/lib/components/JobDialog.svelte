<script lang="ts">
	import type { Job } from '#lib/api/http.ts';
	import JobMonitor from './JobMonitor.svelte';
	import Modal from './Modal.svelte';

	let { title, onfinish }: { title: string; onfinish?: (job: Job) => void } = $props();
	let jobId = $state('');
	let modal: Modal;

	export function show(id: string) {
		jobId = id;
		modal.open();
	}

	export const close = () => modal.close();
</script>

<Modal bind:this={modal} {title}>
	{#snippet children()}
		{#if jobId}<JobMonitor {jobId} {onfinish} />{/if}
	{/snippet}
</Modal>
