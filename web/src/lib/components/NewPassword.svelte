<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import { MAX_LENGTH, MIN_LENGTH, strength } from '#lib/password.ts';
	import Icon from './Icon.svelte';
	import PasswordInput from './PasswordInput.svelte';

	// New password with strength meter, rule checklist and confirmation. Blocks submit until both match.
	let { value = $bindable(), label }: { value: string; label: string } = $props();
	const id = $props.id();

	let confirm = $state('');
	let confirmInput = $state<HTMLInputElement>();
	let left = $state(false);

	const result = $derived(strength(value));
	const matches = $derived(confirm !== '' && confirm === value);
	// Do not nag while the confirmation is still being typed.
	const mismatch = $derived(confirm !== '' && !matches && (left || confirm.length >= value.length));

	const levels = ['', 'weak', 'fair', 'strong', 'veryStrong'] as const;
	// Brighter than the text tokens so a thin bar still reads as colour; the label carries the meaning.
	const bar = ['', 'bg-red-500', 'bg-amber-500', 'bg-lime-500', 'bg-green-500'];
	const tone = ['', 'text-bad', 'text-warn', 'text-ok', 'text-ok'];

	$effect(() => {
		confirmInput?.setCustomValidity(confirm === value ? '' : t('password.mismatch'));
	});
	// The parent clearing the password (form reset) clears the confirmation too.
	$effect(() => {
		if (!value) {
			confirm = '';
			left = false;
		}
	});
</script>

<div class="mb-3">
	<label for="{id}-new" class="mb-1 block text-sm font-medium">{label}</label>
	<PasswordInput id="{id}-new" autocomplete="new-password" required minlength={MIN_LENGTH} maxlength={MAX_LENGTH} aria-describedby="{id}-strength {id}-rules" bind:value />

	<div class="mt-2 grid grid-cols-4 gap-1" aria-hidden="true">
		{#each [1, 2, 3, 4] as step}
			<span class="h-1.5 rounded-full {result.score >= step ? bar[result.score] : 'bg-muted'}"></span>
		{/each}
	</div>
	<p id="{id}-strength" class="mt-1 min-h-5 text-xs font-medium {tone[result.score]}" role="status">
		{result.score ? t('password.strength', { level: t(`password.${levels[result.score] || 'weak'}`) }) : ''}
	</p>

	<ul id="{id}-rules" class="mt-1 grid gap-1 text-xs sm:grid-cols-2">
		{#each ['length', 'unpredictable', 'mixed', 'variety'] as const as rule}
			<li class="flex items-center gap-1.5 {result.rules[rule] ? 'text-ok' : 'text-dim'}">
				{#if result.rules[rule]}<Icon name="check" size={14} />{:else}<span class="grid size-3.5 place-items-center" aria-hidden="true">○</span>{/if}
				<span>{t(`password.rule.${rule}`, { n: MIN_LENGTH })}<span class="sr-only">: {t(result.rules[rule] ? 'password.met' : 'password.unmet')}</span></span>
			</li>
		{/each}
	</ul>
</div>

<div class="mb-3">
	<label for="{id}-confirm" class="mb-1 block text-sm font-medium">{t('password.confirm')}</label>
	<PasswordInput id="{id}-confirm" autocomplete="new-password" required maxlength={MAX_LENGTH} aria-invalid={mismatch} aria-describedby="{id}-match" onblur={() => (left = true)} bind:value={confirm} bind:element={confirmInput} />
	<p id="{id}-match" class="mt-1 flex min-h-5 items-center gap-1.5 text-xs {matches ? 'text-ok' : 'text-bad'}" role="status">
		{#if matches}<Icon name="check" size={14} />{t('password.match')}{:else if mismatch}<Icon name="alert" size={14} />{t('password.mismatch')}{/if}
	</p>
</div>
