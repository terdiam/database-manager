import assert from 'node:assert/strict';
import { device, network } from '../src/lib/client.ts';

for (const [ip, kind] of [['127.0.0.1', 'local'], ['::1', 'local'], ['172.19.0.1', 'private'], ['172.32.0.1', 'public'], ['192.168.1.5', 'private'], ['fd12:3::1', 'private'], ['203.0.113.9', 'public'], ['2001:db8::1', 'public'], ['console', 'console'], ['system', 'system'], ['unknown', 'unknown']] as const) {
	assert.equal(network(ip), kind, ip);
}
const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko)';
assert.equal(device(`${mac} Chrome/140.0.0.0 Safari/537.36`), 'Chrome 140 · macOS');
assert.equal(device(`${mac} Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0`), 'Edge 140 · macOS');
assert.equal(device(`${mac} Version/18.1 Safari/605.1.15`), 'Safari 18 · macOS');
assert.equal(device('Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0'), 'Firefox 130 · Linux');
assert.equal(device('curl/8.7.1'), 'curl 8');
assert.equal(device(''), '');
assert.equal(device('x'.repeat(50)), `${'x'.repeat(40)}…`);
console.log('client ok');
