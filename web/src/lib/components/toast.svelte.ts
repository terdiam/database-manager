export const toasts = $state<{ id: number; text: string }[]>([]);
let next = 0;

export function toast(text: string) {
	const id = ++next;
	toasts.push({ id, text });
}

export function dismiss(id: number) {
	const i = toasts.findIndex((item) => item.id === id);
	if (i >= 0) toasts.splice(i, 1);
}
