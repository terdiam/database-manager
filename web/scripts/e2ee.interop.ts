// Browser code against a running server: node scripts/e2ee.interop.ts http://127.0.0.1:8080
import assert from 'node:assert/strict';
import { CONTENT_TYPE, GZIP_HEADER, HEADER, STREAM_CONTENT_TYPE, sealRequest } from '../src/lib/e2ee.ts';

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

// A compressed request is read by the server: this one is refused for its content, not its form.
const long = JSON.stringify({ email: 'nobody@example.com', password: 'x'.repeat(4000) });
const packed = await sealRequest(info.public_key, 'POST', '/api/auth/login', long, undefined, true);
assert.ok(packed.body!.length < 1000, 'the request was not compressed');
const answered = await fetch(`${base}/api/auth/login`, { method: 'POST', headers: { 'x-dbm-csrf': '1', [HEADER]: packed.header, 'content-type': CONTENT_TYPE }, body: packed.body });
assert.equal(answered.headers.get(GZIP_HEADER), '1');
assert.ok([401, 422].includes((await packed.open(await answered.arrayBuffer())).status));
// The event stream needs a session: pass a test account of a temporary server to check it too.
const { DBM_TEST_EMAIL: email, DBM_TEST_PASSWORD: password } = process.env;
if (email && password) {
	const signIn = await sealRequest(info.public_key, 'POST', '/api/auth/login', JSON.stringify({ email, password }));
	const answer = await fetch(`${base}/api/auth/login`, { method: 'POST', headers: { 'x-dbm-csrf': '1', [HEADER]: signIn.header, 'content-type': CONTENT_TYPE }, body: signIn.body });
	assert.equal((await signIn.open(await answer.arrayBuffer())).status, 200);
	const cookie = answer.headers.get('set-cookie')!.split(';')[0];

	// A large reply to a GET comes back smaller when asked for, and reads the same.
	const read = async (gzip: boolean) => {
		const sealed = await sealRequest(info.public_key, 'GET', '/api/audit', '');
		const response = await fetch(`${base}/api/audit`, { headers: { cookie, [HEADER]: sealed.header, ...(gzip ? { [GZIP_HEADER]: '1' } : {}) } });
		const bytes = await response.arrayBuffer();
		return { size: bytes.byteLength, ...(await sealed.open(bytes)) };
	};
	const [whole, small] = [await read(false), await read(true)];
	assert.equal(small.status, 200);
	assert.deepEqual(JSON.parse(small.body), JSON.parse(whole.body));
	if (whole.body.length >= 1024) assert.ok(small.size < whole.size, 'the reply was not compressed');
	console.log(`GET /api/audit: ${whole.size} -> ${small.size} bytes`);

	const sealed = await sealRequest(info.public_key, 'GET', '/api/events', '');
	const abort = new AbortController();
	const response = await fetch(`${base}/api/events`, { headers: { [HEADER]: sealed.header, cookie }, signal: abort.signal });
	assert.equal(response.headers.get('content-type'), STREAM_CONTENT_TYPE);
	const open = sealed.stream();
	const reader = response.body!.getReader();
	const messages: string[] = [];
	// The first line is "*" at once; the second is an event or the keepalive about 15 s later.
	const started = Date.now();
	while (messages.length < 1) messages.push(...(await open((await reader.read()).value!)));
	assert.ok(Date.now() - started < 3000, 'a proxy is holding the stream back');
	while (messages.length < 2) messages.push(...(await open((await reader.read()).value!)));
	assert.equal(messages[0], '*\n');
	assert.match(messages[1], /^[a-z_]*\n$/);
	abort.abort();
	console.log('event stream ok');
}
console.log(`interop ok (server key ${info.fingerprint}, required=${info.required})`);
