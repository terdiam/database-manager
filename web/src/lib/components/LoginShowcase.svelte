<script lang="ts">
	import logo from '#lib/assets/favicon.svg';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { enter, loop } from '#lib/motion.ts';
	import BrandIcon from './BrandIcon.svelte';
	import Icon, { type IconName } from './Icon.svelte';
	import SplitTitle from './SplitTitle.svelte';

	const engines = ['postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite', 'redis', 'opensearch', 'elasticsearch'];
	const steps: { key: 'source' | 'compress' | 'encrypt' | 'store'; icon: IconName }[] = [
		{ key: 'source', icon: 'database' },
		{ key: 'compress', icon: 'archive' },
		{ key: 'encrypt', icon: 'shield' },
		{ key: 'store', icon: 'cloud' }
	];
	const features: { key: 'schedule' | 'secure' | 'destinations' | 'monitor'; icon: IconName }[] = [
		{ key: 'schedule', icon: 'calendar' },
		{ key: 'secure', icon: 'key' },
		{ key: 'destinations', icon: 'cloud' },
		{ key: 'monitor', icon: 'bell' }
	];
	let selected = $state<(typeof features)[number]['key']>('schedule');
</script>

<div class="flex w-full max-w-xl flex-col gap-7" {@attach enter}>
	<p class="flex items-center gap-2 text-lg font-bold" data-enter><img src={logo} alt="" width="32" height="32" />DB Manager</p>

	<div data-enter>
		<SplitTitle level="h2" text={t('showcase.title')} class="text-3xl font-bold tracking-tight lg:text-4xl" />
		<p class="mt-2 text-dim">{t('showcase.subtitle')}</p>
	</div>

	<!-- How a backup travels. The moving dots are decoration; the labels carry the meaning. -->
	<ol class="flex items-start" aria-label={t('showcase.flow')} data-enter>
		{#each steps as step, i}
			<li class="flex flex-col items-center gap-1.5 text-center text-xs">
				<span class="grid size-12 place-items-center rounded-2xl border border-line bg-surface text-brand shadow-sm" {@attach loop({ transform: ['translateY(0px)', 'translateY(-6px)', 'translateY(0px)'] }, 3.2, i * 0.35)}>
					<Icon name={step.icon} size={22} />
				</span>
				<span class="w-16 text-dim">{t(`showcase.step.${step.key}`)}</span>
			</li>
			{#if i < steps.length - 1}
				<li class="relative mt-6 h-px flex-1 bg-line" aria-hidden="true">
					<span class="absolute inset-0" {@attach loop({ transform: ['translateX(0%)', 'translateX(100%)'], opacity: [0, 1, 0] }, 1.8, i * 0.45)}>
						<span class="absolute -top-1 left-0 size-2 rounded-full bg-brand"></span>
					</span>
				</li>
			{/if}
		{/each}
	</ol>

	<div data-enter>
		<div class="grid grid-cols-2 gap-2 sm:grid-cols-4" role="group" aria-label={t('showcase.features')}>
			{#each features as feature (feature.key)}
				<button type="button" aria-pressed={selected === feature.key} onclick={() => (selected = feature.key)} onpointerenter={() => (selected = feature.key)}
					class="flex flex-col items-center gap-1.5 rounded-xl border p-3 text-xs font-medium transition hover:-translate-y-0.5 motion-reduce:transform-none {selected === feature.key ? 'border-brand bg-surface text-brand shadow-md' : 'border-line bg-surface/60'}">
					<Icon name={feature.icon} size={20} />{t(`showcase.${feature.key}.title`)}
				</button>
			{/each}
		</div>
		<!-- Every text shares one grid cell, so the box is always as tall as the longest one: no jumping. -->
		<div class="mt-3 grid rounded-xl border border-line bg-surface p-3 text-sm" role="status">
			{#each features as feature (feature.key)}
				<p class="col-start-1 row-start-1 {selected === feature.key ? '' : 'invisible'}">{t(`showcase.${feature.key}.text`)}</p>
			{/each}
		</div>
	</div>

	<div data-enter>
		<p class="mb-2 text-xs font-semibold text-dim uppercase">{t('showcase.engines')}</p>
		<ul class="flex flex-wrap gap-2">
			{#each engines as engine}
				<li class="transition hover:-translate-y-1 hover:scale-110 motion-reduce:transform-none" title={engine}>
					<BrandIcon name={engine} size={24} /><span class="sr-only">{engine}</span>
				</li>
			{/each}
		</ul>
	</div>
</div>
