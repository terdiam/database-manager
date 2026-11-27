<script lang="ts" module>
	export type Field = { label: string; type?: 'text' | 'number' | 'password' | 'email' | 'url' | 'textarea'; value?: string; help?: string; options?: string[] };
</script>

<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Status } from '#lib/api/index.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import BrandIcon from './BrandIcon.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import DataTable from './DataTable.svelte';
	import FormField from './FormField.svelte';
	import Modal from './Modal.svelte';
	import PageHeader from './PageHeader.svelte';
	import StatusBadge from './StatusBadge.svelte';
	import { toast } from './toast.svelte';

	let {
		title, description, addLabel, columns, rows, fields, testLabel = '', before, brandColumn = -1
	}: {
		title: string; description: string; addLabel: string; columns: string[];
		rows: { id: string; cells: string[]; status?: Status }[];
		fields: Field[]; testLabel?: string; before?: Snippet; brandColumn?: number;
	} = $props();

	let modal: Modal;
	let confirm: ConfirmDialog;
	let removing = $state('');

	const headers = $derived([...columns, t('col.actions')]);

	function remove(name: string) {
		removing = name;
		confirm.open();
	}
</script>

<PageHeader {title} {description}>
	{#snippet actions()}<button type="button" class="btn btn-primary" onclick={() => modal.open()}>{addLabel}</button>{/snippet}
</PageHeader>

<DataTable label={title} columns={headers} {rows}>
	{#snippet cell(row, i)}
		{#if i === headers.length - 1}
			<span class="flex flex-wrap justify-end gap-1">
				{#if testLabel}<button type="button" class="btn" onclick={() => toast(t('toast.test', { name: row.cells[0] }))}>{testLabel}</button>{/if}
				<button type="button" class="btn btn-danger" onclick={() => remove(row.cells[0])}>{t('action.delete')}</button>
			</span>
		{:else if i === row.cells.length && row.status}<StatusBadge status={row.status} />
		{:else if i === brandColumn}<span class="inline-flex items-center gap-2"><BrandIcon name={row.cells[i]} />{row.cells[i]}</span>
		{:else if i === 0}<strong>{row.cells[0]}</strong>
		{:else}{row.cells[i]}{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={addLabel}>
	{#snippet children(close)}
		<form onsubmit={(e) => { e.preventDefault(); e.currentTarget.reset(); close(); toast(t('toast.saved')); }}>
			{@render before?.()}
			{#each fields as field (field.label)}
				<FormField label={field.label} help={field.help}>
					{#snippet children(a)}
						{#if field.options}<select {...a} class="input">{#each field.options as option}<option>{option}</option>{/each}</select>
						{:else if field.type === 'textarea'}<textarea {...a} class="input py-2" rows="4" required></textarea>
						{:else}<input {...a} class="input" type={field.type ?? 'text'} value={field.value ?? ''} autocomplete="off" required />{/if}
					{/snippet}
				</FormField>
			{/each}
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<ConfirmDialog
	bind:this={confirm}
	title={t('action.delete')}
	message={t('confirm.delete', { name: removing })}
	confirmLabel={t('action.delete')}
	onconfirm={() => toast(t('toast.demoDelete'))}
/>
