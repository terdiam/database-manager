export type Open = (onLine: (line: string) => void, signal: AbortSignal) => Promise<void>;

type Options = {
	/** Reload anyway this often, for what no event announces (an agent going silent). */
	every?: number;
	/** Also load right away; otherwise the first load stays with the caller. */
	now?: boolean;
};

/**
 * One server event stream shared by everything on screen. `subscribe` calls `load` when one of
 * `topics` changes, when the stream (re)connects, and every `every` ms as a safety net. Hidden
 * tabs close the stream and do nothing.
 */
export function createHub(open: Open, { delay = 500, retry = 1000, silence = 40_000 } = {}) {
	type Listener = { topics: string[]; touch: () => void };
	const listeners = new Set<Listener>();
	let controller: AbortController | undefined;
	let timer: ReturnType<typeof setTimeout>;
	let wait = retry;
	const visible = () => document.visibilityState === 'visible';

	function connect() {
		clearTimeout(timer);
		if (controller || !listeners.size || !visible()) return;
		const mine = (controller = new AbortController());
		// The server speaks every 15 s: a long silence means the connection is dead.
		let watchdog = setTimeout(() => mine.abort(), silence);
		open((line) => {
			clearTimeout(watchdog);
			watchdog = setTimeout(() => mine.abort(), silence);
			wait = retry;
			for (const l of listeners) if (line === '*' || l.topics.includes(line)) l.touch();
		}, mine.signal)
			.catch(() => {})
			.finally(() => {
				clearTimeout(watchdog);
				if (controller !== mine) return; // closed on purpose
				controller = undefined;
				timer = setTimeout(connect, wait);
				wait = Math.min(wait * 2, 30_000);
			});
	}

	function disconnect() {
		clearTimeout(timer);
		const current = controller;
		controller = undefined;
		current?.abort();
		wait = retry;
	}

	const visibility = () => (visible() ? connect() : disconnect());

	/** Returns the stop function, so it can be returned straight from `onMount`. */
	return function subscribe(topics: string[], load: () => unknown, { every = 60_000, now = false }: Options = {}) {
		let pending: ReturnType<typeof setTimeout> | undefined;
		let busy = false;
		let again = false;
		let stopped = false;

		async function run() {
			pending = undefined;
			if (stopped) return;
			if (busy) {
				again = true;
				return;
			}
			busy = true;
			try {
				await load();
			} catch {
				// The loader shows its own errors.
			} finally {
				busy = false;
				if (again) {
					again = false;
					touch();
				}
			}
		}
		// A burst of events becomes one load.
		function touch() {
			if (!stopped) pending ??= setTimeout(run, delay);
		}

		const listener = { topics, touch };
		if (!listeners.size) document.addEventListener('visibilitychange', visibility);
		listeners.add(listener);
		const fallback = setInterval(() => visible() && run(), every);
		connect();
		if (now) run();

		return () => {
			stopped = true;
			clearTimeout(pending);
			clearInterval(fallback);
			listeners.delete(listener);
			if (!listeners.size) {
				document.removeEventListener('visibilitychange', visibility);
				disconnect();
			}
		};
	};
}
