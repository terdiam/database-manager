import assert from 'node:assert/strict';
import { bounds, emptyDate, inDate } from '../src/lib/daterange.ts';

const now = new Date(2026, 2, 3, 10, 0);
const at = (m: number, d: number, h = 12) => new Date(2026, m, d, h).toISOString();
assert.deepEqual(bounds({ ...emptyDate(), preset: '7d' }, now), ['2026-02-25', '2026-03-03']);
assert.ok(inDate(at(2, 3, 23), { ...emptyDate(), preset: 'today' }, now));
assert.ok(!inDate(at(2, 2, 23), { ...emptyDate(), preset: 'today' }, now));
assert.ok(inDate(at(0, 15, 0), { ...emptyDate(), preset: 'date', date: '2026-01-15' }, now));
assert.ok(inDate(at(0, 31), { ...emptyDate(), preset: 'range', from: '2026-01-01', to: '2026-01-31' }, now));
assert.ok(!inDate(at(1, 1), { ...emptyDate(), preset: 'range', from: '2026-01-01', to: '2026-01-31' }, now));
assert.ok(inDate(at(1, 1), { ...emptyDate(), preset: 'range', from: '2026-01-01' }, now));
assert.ok(inDate(at(1, 1), emptyDate(), now));
console.log('daterange ok');
