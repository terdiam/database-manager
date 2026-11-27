import assert from 'node:assert/strict';
import { createHub } from '../src/lib/live.ts';

const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
(globalThis as { document?: unknown }).document = page;
const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));
const show = (state: string) => {
	page.visibilityState = state;
	page.dispatchEvent(new Event('visibilitychange'));
};

// A fake server stream that the test can write to and drop.
let opened = 0;
let send = (_line: string) => {};
let drop = () => {};
let aborted = false;
const subscribe = createHub(
	(onLine, signal) =>
		new Promise<void>((done) => {
			opened++;
			aborted = false;
			send = onLine;
			drop = done;
			signal.addEventListener('abort', () => {
				aborted = true;
				done();
			});
		}),
	{ delay: 10, retry: 20, silence: 150 }
);

let jobs = 0;
let agents = 0;
let running = 0;
let overlapped = false;
const stopJobs = subscribe(['jobs'], async () => {
	overlapped ||= running > 0;
	running++;
	jobs++;
	await sleep(30);
	running--;
	throw new Error('a failed load must not stop later ones');
});
const stopAgents = subscribe(['agents'], () => void agents++);
assert.equal(opened, 1, 'one stream for all subscribers');

send('jobs');
send('jobs');
send('jobs');
await sleep(25);
assert.deepEqual([jobs, agents], [1, 0], 'a burst is one load, only for its topic');
send('jobs');
await sleep(80);
assert.equal(jobs, 2, 'an event during a load runs it once more');
assert.ok(!overlapped, 'loads never overlap');

send('*');
await sleep(60);
assert.deepEqual([jobs, agents], [3, 1], 'a star reaches everyone');

drop();
await sleep(60);
assert.equal(opened, 2, 'reconnects after the server closes');

await sleep(220);
assert.ok(opened >= 3, 'reconnects after silence');

show('hidden');
assert.ok(aborted, 'hidden tab closes the stream');
const before = opened;
await sleep(60);
assert.equal(opened, before, 'and stays closed');
show('visible');
assert.equal(opened, before + 1, 'opens again when visible');

stopJobs();
assert.ok(!aborted, 'still needed by another subscriber');
stopAgents();
assert.ok(aborted, 'closes with the last subscriber');

// The safety net works without any event.
let ticks = 0;
const stop = subscribe(['x'], () => void ticks++, { every: 20, now: true });
assert.equal(ticks, 1, 'now');
await sleep(70);
assert.ok(ticks >= 3, 'fallback');
stop();

console.log('live ok');
