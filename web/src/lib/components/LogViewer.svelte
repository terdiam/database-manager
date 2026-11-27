<script lang="ts">
	let { lines, label }: { lines: string[]; label: string } = $props();
	const tone = (line: string) => (line.startsWith('[error]') ? 'text-bad' : line.startsWith('[ok]') ? 'text-ok' : '');
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable log needs keyboard access) -->
<div class="max-h-80 overflow-auto rounded-lg bg-muted p-3 font-mono text-xs leading-5" role="log" aria-label={label} tabindex="0">
	{#each lines as line}<div class="break-all whitespace-pre-wrap {tone(line)}">{line}</div>{/each}
</div>
