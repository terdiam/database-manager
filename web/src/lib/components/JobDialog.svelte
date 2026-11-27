<script lang="ts">
	import JobMonitor from './JobMonitor.svelte';
	import Modal from './Modal.svelte';

	let { title }: { title: string } = $props();
	let jobId = $state('');
	let modal: Modal;

	export function show(id: string) {
		jobId = id;
		modal.open();
	}
</script>

<Modal bind:this={modal} {title}>
	{#snippet children()}
		{#if jobId}<JobMonitor {jobId} />{/if}
	{/snippet}
</Modal>
