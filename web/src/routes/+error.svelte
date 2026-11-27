<script lang="ts">
	import { page } from '$app/state';
	import Illustration from '#lib/components/Illustration.svelte';
	import { t } from '#lib/i18n/index.svelte.ts';
</script>

<main class="grid min-h-dvh place-items-center p-4 text-center">
	<div>
		<Illustration name="empty" class="mx-auto w-56" />
		<h1 class="text-xl font-semibold">{page.status === 404 ? t('error.not_found') : t('error.network')}</h1>
		<button type="button" class="btn btn-primary mt-4" onclick={() => location.reload()}>{t('action.retry')}</button>
	</div>
</main>
