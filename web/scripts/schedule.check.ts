// Self-check: pnpm test
import assert from 'node:assert/strict';
import { expiresAt, fromCron, newRetention, newSchedule, retentionOf, toCron, toPlan } from '../src/lib/schedule.ts';

const from = new Date(2026, 0, 31, 12, 0);
assert.equal(expiresAt('months', 1, from).getDate(), 28);
assert.equal(expiresAt('days', 1, from).getMonth(), 1);

// What the form writes it must read back the same.
for (const schedule of [
	newSchedule('hours', { every: 6 }),
	newSchedule('daily', { time: '01:30' }),
	newSchedule('weekly', { time: '23:00', weekday: 5 }),
	newSchedule('monthly', { day: 31 }),
	newSchedule('yearly', { month: 3, day: 5 })
]) {
	const back = fromCron(toCron(schedule));
	assert.equal(back.kind, schedule.kind);
	assert.equal(toCron(back), toCron(schedule));
}
assert.equal(toCron(newSchedule('daily', { time: '01:30' })), '30 1 * * *');
assert.equal(toCron(newSchedule('manual')), null);
assert.equal(fromCron('*/5 * * * *').kind, 'cron');
assert.equal(fromCron(null).kind, 'manual');

const plan = toPlan(newSchedule('manual'), 'UTC', newRetention({ unit: 'gfs', daily: 3 }));
assert.deepEqual([plan.retention_unit, plan.retention_value, plan.keep_daily], ['gfs', null, 3]);
assert.equal(toPlan(newSchedule('manual'), 'UTC', newRetention({ unit: 'all' })).retention_unit, null);
assert.equal(retentionOf(plan).daily, 3);
console.log('schedule ok');
