<script lang="ts">
	let { value, label }: { value: number; label: string } = $props();
</script>

<div class="flex items-center gap-2">
	<progress class="h-2 w-full accent-brand" max="100" {value} aria-label={label}></progress>
	<span class="w-10 text-right text-xs tabular-nums text-dim">{value}%</span>
</div>
