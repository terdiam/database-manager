import assert from 'node:assert/strict';
import { strength } from '../src/lib/password.ts';

assert.equal(strength('').score, 0);
assert.equal(strength('Ab1!').score, 1); // short
assert.equal(strength('aaaaaaaaaaaaaaaa').score, 1); // repeated
assert.equal(strength('Password12345!').score, 1); // common
assert.equal(strength('kudalautbiru').score, 2);
assert.equal(strength('KudaLaut-biru').score, 3);
assert.equal(strength('KudaLaut-biru-77').score, 4);
console.log('password ok');
