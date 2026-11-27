<script lang="ts">
	import { revealWords } from '#lib/motion.ts';

	let { text, level = 'h2', class: cls = '' }: { text: string; level?: 'h1' | 'h2' | 'h3'; class?: string } = $props();
</script>

<!-- Filled by the attachment, which reruns when `text` changes (e.g. language switch). -->
<svelte:element this={level} class={cls} {@attach revealWords(() => text)}></svelte:element>
