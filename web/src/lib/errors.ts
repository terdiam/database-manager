import { ApiError } from './api/http.ts';
import { t, type Key } from './i18n/index.svelte.ts';
import { messages } from './i18n/messages.ts';

export function errorText(error: unknown): string {
	const key = `error.${error instanceof ApiError ? error.code : 'internal'}`;
	return t((key in messages ? key : 'error.internal') as Key);
}
