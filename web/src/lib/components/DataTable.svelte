<script lang="ts" generics="T extends { id: string }">
	import type { Snippet } from 'svelte';
	import EmptyState from './EmptyState.svelte';

	let {
		label,
		columns,
		rows,
		cell
	}: {
		label: string;
		columns: string[];
		rows: T[];
		cell: Snippet<[T, number]>;
	} = $props();
</script>

{#if rows.length === 0}
	<EmptyState />
{:else}
	<!-- svelte-ignore a11y_no_noninteractive_tabindex (keyboard users must be able to scroll the table) -->
	<div class="table-wrap card p-0" role="region" aria-label={label} tabindex="0">
		<table class="table">
			<thead>
				<tr>{#each columns as column}<th scope="col">{column}</th>{/each}</tr>
			</thead>
			<tbody>
				{#each rows as row (row.id)}
					<tr>
						{#each columns as column, i}<td data-label={column}>{@render cell(row, i)}</td>{/each}
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
