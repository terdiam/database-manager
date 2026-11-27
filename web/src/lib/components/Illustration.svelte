<script lang="ts">
	let { name, class: cls = '' }: { name: 'empty' | 'backup'; class?: string } = $props();
</script>

<!-- Decorative, drawn with theme tokens so it follows light/dark and the accent. -->
<svg viewBox="0 0 240 150" class={cls} aria-hidden="true" fill="none" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
	{#if name === 'empty'}
		<ellipse cx="120" cy="132" rx="70" ry="8" class="fill-muted" />
		<path d="M70 70h100l-8 55H78z" class="fill-surface stroke-line" />
		<path d="M62 56h116v14H62z" class="fill-muted stroke-line" />
		<path d="M103 90h34" class="stroke-dim" />
		<circle cx="150" cy="34" r="14" class="stroke-brand" />
		<path d="M161 45l11 11M96 36l-8-8M112 30v-10" class="stroke-brand" />
	{:else}
		<ellipse cx="55" cy="40" rx="30" ry="11" class="fill-muted stroke-brand" />
		<path d="M25 40v60c0 6 13 11 30 11s30-5 30-11V40M25 70c0 6 13 11 30 11s30-5 30-11" class="stroke-brand" />
		<path d="M96 75h46M132 64l12 11-12 11" class="stroke-ok" stroke-dasharray="6 6" />
		<path d="M170 108a22 22 0 0 1 0-44 28 28 0 0 1 52 8 18 18 0 0 1-4 36z" class="fill-muted stroke-brand" />
		<path d="M184 88l8 8 15-16" class="stroke-ok" />
		<path d="M20 130h200" class="stroke-line" />
	{/if}
</svg>
