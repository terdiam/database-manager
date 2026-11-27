// Self-check: pnpm test
import assert from 'node:assert/strict';
import { expiresAt, newSchedule, nextRuns } from '../src/lib/schedule.ts';

const from = new Date(2026, 0, 31, 12, 0);
assert.equal(expiresAt('months', 1, from).getDate(), 28);
assert.equal(expiresAt('days', 1, from).getMonth(), 1);
assert.deepEqual(nextRuns(newSchedule('hours', { every: 6 }), 2, from).map((d) => d.getHours()), [18, 0]);
assert.equal(nextRuns(newSchedule('monthly', { day: 31 }), 1, from)[0].getMonth(), 2);
assert.equal(nextRuns(newSchedule('yearly', { month: 3, day: 5 }), 1, from)[0].getMonth(), 2);
assert.equal(nextRuns(newSchedule('daily', { time: '01:00' }), 1, from)[0].getDate(), 1);
console.log('schedule ok');
