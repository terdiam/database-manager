<script lang="ts" module>
	const paths = {
		dashboard: 'M4 4h7v7H4zM13 4h7v4h-7zM13 11h7v9h-7zM4 14h7v6H4z',
		database: 'M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3',
		key: 'M15 7a4 4 0 1 1-3.9 5H8v3H5v3H2v-4l6.1-6.1M16 8h.01',
		calendar: 'M4 5h16v16H4zM4 10h16M8 3v4M16 3v4',
		archive: 'M3 4h18v5H3zM5 9v11h14V9M10 13h4',
		list: 'M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01',
		cloud: 'M7 19a5 5 0 1 1 .9-9.9A6 6 0 0 1 19.5 11 4 4 0 0 1 18 19z',
		server: 'M4 4h16v7H4zM4 13h16v7H4zM8 7.5h.01M8 16.5h.01',
		bell: 'M6 9a6 6 0 0 1 12 0c0 6 3 7 3 7H3s3-1 3-7M10 20a2 2 0 0 0 4 0',
		users: 'M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM2 21a7 7 0 0 1 14 0M16 4a4 4 0 0 1 0 7M18 15a7 7 0 0 1 4 6',
		settings: 'M4 7h10M18 7h2M4 17h2M10 17h10M14 4v6M6 14v6',
		menu: 'M4 6h16M4 12h16M4 18h16',
		close: 'M6 6l12 12M18 6L6 18',
		check: 'M5 13l4 4L19 7',
		alert: 'M12 3l10 18H2zM12 10v4M12 17.5h.01',
		clock: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2',
		spin: 'M21 12a9 9 0 1 1-9-9',
		plus: 'M12 5v14M5 12h14',
		play: 'M7 4l13 8-13 8z',
		restore: 'M3 12a9 9 0 1 0 3-6.7M3 4v5h5',
		download: 'M12 3v12M7 11l5 5 5-5M4 21h16',
		user: 'M12 12a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM4 21a8 8 0 0 1 16 0',
		book: 'M12 6c-2-2-5-2-9-1v14c4-1 7-1 9 1 2-2 5-2 9-1V5c-4-1-7-1-9 1zM12 6v14',
		map: 'M3 6l6-3 6 3 6-3v15l-6 3-6-3-6 3zM9 3v15M15 6v15',
		palette: 'M12 3a9 9 0 1 0 0 18c1.5 0 2-1 1.5-2.2-.6-1.4.2-2.8 1.8-2.8H18a3 3 0 0 0 3-3c0-5.500-4-10-9-10zM7.5 12h.01M9.5 8h.01M14.500 8h.01',
		sun: 'M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.400 1.4M18.4 5.6L17 7M7 17l-1.400 1.4',
		globe: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18',
		logout: 'M10 4H4v16h6M9 12h12M17 8l4 4-4 4',
		chevron: 'M9 5l7 7-7 7',
		shield: 'M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM8.5 12l2.5 2.5 4.5-5',
		eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
		eyeOff: 'M3 3l18 18M10.6 5.1A10.9 10.9 0 0 1 12 5c6.5 0 10 7 10 7a17.6 17.6 0 0 1-3 3.9M6.6 6.6A17.2 17.2 0 0 0 2 12s3.5 7 10 7a10.7 10.7 0 0 0 5.4-1.5M9.9 9.9a3 3 0 0 0 4.2 4.2',
		search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM21 21l-5-5',
		moon: 'M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z',
		updown: 'M8 9l4-4 4 4M8 15l4 4 4-4'
	};
	export type IconName = keyof typeof paths;
</script>

<script lang="ts">
	let { name, size = 18, class: cls = '' }: { name: IconName; size?: number; class?: string } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 {cls}" aria-hidden="true">
	<path d={paths[name]} />
</svg>
