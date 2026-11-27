<script lang="ts">
	let { name, size = 32 }: { name: string; size?: number } = $props();
	const initials = $derived(name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join(''));
</script>

<span class="inline-grid shrink-0 place-items-center rounded-full bg-muted font-semibold text-dim" style:width="{size}px" style:height="{size}px" style:font-size="{size * 0.38}px" aria-hidden="true">{initials}</span>
