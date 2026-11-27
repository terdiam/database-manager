// Browser code against a running server: node scripts/e2ee.interop.ts http://127.0.0.1:8080
import assert from 'node:assert/strict';
import { CONTENT_TYPE, HEADER, sealRequest } from '../src/lib/e2ee.ts';

const base = process.argv[2] ?? 'http://127.0.0.1:8080';
const info = await (await fetch(`${base}/api/e2ee`)).json();

async function call(method: string, path: string, body = '') {
	const sealed = await sealRequest(info.public_key, method, path, body);
	const response = await fetch(base + path, {
		method,
		headers: { 'x-dbm-csrf': '1', [HEADER]: sealed.header, ...(sealed.body ? { 'content-type': CONTENT_TYPE } : {}) },
		body: sealed.body
	});
	assert.equal(response.headers.get('content-type'), CONTENT_TYPE, `${method} ${path}`);
	return sealed.open(await response.arrayBuffer());
}

const state = await call('GET', '/api/auth/state');
assert.equal(state.status, 200);
assert.ok('setup_required' in JSON.parse(state.body));

// An error must come back encrypted too, with the real status inside.
const audit = await call('GET', '/api/audit?q=a%20b&action=login');
assert.equal(audit.status, 401);
const login = await call('POST', '/api/auth/login', JSON.stringify({ email: 'nobody@example.com', password: 'wrong-password-1' }));
assert.ok([401, 422].includes(login.status), String(login.status));
assert.ok(JSON.parse(login.body).error);
console.log(`interop ok (server key ${info.fingerprint}, required=${info.required})`);
