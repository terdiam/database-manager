<script lang="ts">
	import { onMount } from 'svelte';
	import '../app.css';
	import { smoothScroll } from '#lib/motion.ts';
	import Toaster from '#lib/components/Toaster.svelte';
	import { locale } from '#lib/i18n/index.svelte.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	onMount(smoothScroll);

	$effect(() => {
		document.documentElement.lang = locale.current;
	});
</script>

{@render children()}
<Toaster />
