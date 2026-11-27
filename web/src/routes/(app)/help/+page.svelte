<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { t } from '#lib/i18n/index.svelte.ts';
</script>

<PageHeader title={t('menu.docs')} description={t('help.docsDesc')} />

<ol class="space-y-3">
	{#each ['database', 'destination', 'backup', 'monitor'] as const as step, i}
		<li class="card">
			<SplitTitle text="{i + 1}. {t(`tour.${step}.title`)}" class="font-semibold" />
			<p class="mt-1 text-sm">{t(`tour.${step}.text`)}</p>
		</li>
	{/each}
</ol>
