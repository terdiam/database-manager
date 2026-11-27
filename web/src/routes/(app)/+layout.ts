import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';
import { request, type AuthState } from '#lib/api/http.ts';
import { session } from '#lib/session.svelte.ts';

// The server enforces permissions; this only avoids showing pages that would fail.
const adminOnly = ['/users', '/audit', '/settings', '/notifications'];

export async function load({ url }) {
	const { user } = await request<AuthState>('GET', '/auth/state');
	session.user = user;
	if (!user) redirect(307, resolve('login'));
	if (user.role !== 'admin' && adminOnly.some((path) => url.pathname.startsWith(path))) redirect(307, resolve('/(app)'));
}
