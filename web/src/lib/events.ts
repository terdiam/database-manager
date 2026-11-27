import { openStream } from './api/http.ts';
import { createHub } from './live.ts';

/** Reload when the server reports a change: `onMount(() => live(['agents'], load))`. */
export const live = createHub((onLine, signal) => openStream('/events', onLine, signal));
