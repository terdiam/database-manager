import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { CONTENT_TYPE, HEADER, sealRequest } from '../e2ee.ts';

/** `code` matches the server's error codes and is translated as `error.<code>`. */
export class ApiError extends Error {
	constructor(readonly code: string) {
		super(code);
	}
}

export type E2eeInfo = { required: boolean; public_key: string; fingerprint: string };

let encryption: Promise<E2eeInfo> | undefined;

/** Whether the server wants encrypted payloads, and the key to encrypt to. */
export function e2eeInfo(): Promise<E2eeInfo> {
	return (encryption ??= fetch('/api/e2ee')
		.then((response) => (response.ok ? (response.json() as Promise<E2eeInfo>) : Promise.reject(new ApiError('internal'))))
		.catch((error) => {
			encryption = undefined; // ask again next time
			throw error instanceof ApiError ? error : new ApiError('network');
		}));
}

type Raw = { status: number; data: any }; // eslint-disable-line @typescript-eslint/no-explicit-any

const parse = (body: string) => {
	try {
		return JSON.parse(body);
	} catch {
		return undefined;
	}
};

async function send(method: string, url: string, init: RequestInit): Promise<Response> {
	try {
		return await fetch(url, { ...init, method });
	} catch {
		throw new ApiError('network');
	}
}

async function plain(method: string, url: string, body: string): Promise<Raw> {
	const response = await send(method, url, {
		headers: { 'x-dbm-csrf': '1', ...(body ? { 'content-type': 'application/json' } : {}) },
		body: body || undefined
	});
	return { status: response.status, data: parse(await response.text()) };
}

async function encrypted(method: string, url: string, body: string, key: string): Promise<Raw> {
	// WebCrypto only exists on https and localhost.
	if (!window.isSecureContext) throw new ApiError('insecure_context');
	const sealed = await sealRequest(key, method, url, body);
	const response = await send(method, url, {
		headers: {
			'x-dbm-csrf': '1',
			[HEADER]: sealed.header,
			...(sealed.body ? { 'content-type': CONTENT_TYPE } : {})
		},
		body: sealed.body
	});
	if (response.headers.get('content-type') === CONTENT_TYPE) {
		const reply = await sealed.open(await response.arrayBuffer()).catch(() => {
			throw new ApiError('e2ee_invalid');
		});
		return { status: reply.status, data: parse(reply.body) };
	}
	// Refused before decryption. Plain text is never accepted as success.
	if (response.ok) throw new ApiError('e2ee_invalid');
	return { status: response.status, data: parse(await response.text()) };
}

export async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
	const config = await e2eeInfo();
	// Exactly what the browser will put on the request line: it is part of what gets signed.
	const target = new URL(`/api${path}`, location.origin);
	const url = target.pathname + target.search;
	const json = body ? JSON.stringify(body) : '';
	const { status, data } = config.required
		? await encrypted(method, url, json, config.public_key)
		: await plain(method, url, json);

	if (status >= 200 && status < 300) return data as T;
	const code: string = data?.error ?? 'internal';
	// The setting or key changed while the app was open: fetch it again next time.
	if (code === 'e2ee_required' || code === 'e2ee_invalid') encryption = undefined;
	// The session ended while the app was open.
	if (code === 'unauthorized') await goto(resolve('login'), { invalidateAll: true });
	throw new ApiError(code);
}

export type Role = 'admin' | 'operator' | 'viewer';
export type User = { id: string; name: string; email: string; role: Role };
export type AuthState = { setup_required: boolean; user: User | null; turnstile_site_key: string | null };
export type Agent = { id: string; name: string; version: string; last_seen_at: string; online: boolean; fingerprint: string | null; key: string | null };
export type AuditEntry = { id: number; created_at: string; user_name: string; action: string; target: string; ip: string; ok: boolean };
export type AuditPage = { items: AuditEntry[]; next: number | null };

export type DbServer = { id: string; name: string; engine: 'postgresql'; agent_id: string; agent: string; host: string; port: number; username: string };
export type Destination = { id: string; name: string; kind: 's3'; agent_id: string; agent: string; endpoint: string; bucket: string; prefix: string };
export type Backup = { id: string; name: string; server_id: string; server: string; database_name: string; destination_id: string; destination: string; compression: string; level: number };
export type Snapshot = { id: string; server_name: string; database_name: string; destination: string; agent_id: string; object_key: string; size: number; sha256: string; compression: string; created_at: string };
export type JobKind = 'backup' | 'restore' | 'test_database' | 'test_destination' | 'delete_snapshot';
export type JobStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
export type Job = { id: string; kind: JobKind; status: JobStatus; target: string; agent: string; bytes: number; error: string | null; created_by: string; cancel_requested: boolean; created_at: string; started_at: string | null; finished_at: string | null };
export type JobLine = { id: number; level: string; message: string };
export type Page<T> = { items: T[]; next: string | null };
export type Queued = { job_id: string };
