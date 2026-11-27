<script lang="ts">
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	function save(event: SubmitEvent) {
		event.preventDefault();
		toast(t('toast.saved'));
	}
</script>

<PageHeader title={t('nav.settings')} />

<form class="card" onsubmit={save}>
	<label class="mb-3 block text-sm font-medium">{t('settings.timezone')}
		<select class="input mt-1"><option>Asia/Jakarta</option><option>Asia/Makassar</option><option>Asia/Jayapura</option><option>UTC</option></select>
	</label>
	<label class="mb-3 block text-sm font-medium">{t('settings.parallel')}
		<input class="input mt-1" type="number" min="1" max="16" value="2" required />
	</label>
	<label class="mb-4 block text-sm font-medium">{t('settings.logDays')}
		<input class="input mt-1" type="number" min="1" max="365" value="30" required />
	</label>
	<button type="submit" class="btn btn-primary">{t('action.save')}</button>
</form>
