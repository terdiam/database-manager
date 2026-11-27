import type { User } from './api/http.ts';

/** Filled by the (app) layout load; pages inside it can rely on `user`. */
export const session = $state<{ user: User | null }>({ user: null });

export const isAdmin = () => session.user?.role === 'admin';
