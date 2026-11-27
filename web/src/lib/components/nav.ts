import type { Key } from '#lib/i18n/messages.ts';
import type { IconName } from './Icon.svelte';

export const nav: { group: Key | null; items: { href: string; label: Key; icon: IconName; admin?: boolean }[] }[] = [
	{ group: null, items: [{ href: '/', label: 'nav.dashboard', icon: 'dashboard' }] },
	{ group: 'nav.sources', items: [
		{ href: '/databases', label: 'nav.databases', icon: 'database' },
		{ href: '/ssh', label: 'nav.ssh', icon: 'key' }
	] },
	{ group: 'nav.backup', items: [
		{ href: '/backups', label: 'nav.backups', icon: 'calendar' },
		{ href: '/snapshots', label: 'nav.snapshots', icon: 'archive' },
		{ href: '/jobs', label: 'nav.jobs', icon: 'list' }
	] },
	{ group: 'nav.system', items: [
		{ href: '/destinations', label: 'nav.destinations', icon: 'cloud' },
		{ href: '/agents', label: 'nav.agents', icon: 'server' },
		{ href: '/notifications', label: 'nav.notifications', icon: 'bell', admin: true },
		{ href: '/users', label: 'nav.users', icon: 'users', admin: true },
		{ href: '/audit', label: 'nav.audit', icon: 'shield', admin: true },
		{ href: '/settings', label: 'nav.settings', icon: 'settings', admin: true }
	] }
];
