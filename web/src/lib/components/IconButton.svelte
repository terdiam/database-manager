<script lang="ts">
	import Icon, { type IconName } from './Icon.svelte';

	// An action shown as an icon. `label` is its accessible name and the hint shown on hover or
	// keyboard focus. The hint is a popover so table scrolling cannot clip it; it is placed by
	// script because anchor positioning is not available everywhere yet.
	let {
		icon,
		label,
		href,
		danger = false,
		pressed,
		onclick
	}: { icon: IconName; label: string; href?: string; danger?: boolean; pressed?: boolean; onclick?: () => void } = $props();

	let trigger = $state<HTMLElement>();
	let hint = $state<HTMLElement>();
	let open = $state(false);
	let timer: ReturnType<typeof setTimeout>;

	function show() {
		clearTimeout(timer);
		if (!trigger?.isConnected || !hint) return;
		hint.showPopover();
		const at = trigger.getBoundingClientRect();
		const { offsetWidth: width, offsetHeight: height } = hint;
		const left = at.left + (at.width - width) / 2;
		hint.style.left = `${Math.max(4, Math.min(left, document.documentElement.clientWidth - width - 4))}px`;
		hint.style.top = `${at.top >= height + 10 ? at.top - height - 6 : at.bottom + 6}px`;
		open = true;
	}

	function hide() {
		clearTimeout(timer);
		if (open) hint?.hidePopover();
		open = false;
	}

	// A short grace period lets the pointer move onto the hint itself.
	const leave = () => (timer = setTimeout(hide, 120));
	const click = () => {
		hide();
		onclick?.();
	};
	const focus = () => trigger?.matches(':focus-visible') && show();
</script>

<!-- A scrolled hint would float away from its button, and Escape must dismiss it. -->
<svelte:window onscrollcapture={() => open && hide()} onkeydown={(e) => e.key === 'Escape' && hide()} />

<svelte:element
	this={href ? 'a' : 'button'}
	bind:this={trigger}
	type={href ? undefined : 'button'}
	{href}
	role={href ? 'link' : 'button'}
	class="btn size-10 px-0 {danger ? 'btn-danger' : ''} {pressed ? 'border-brand bg-muted' : ''}"
	aria-label={label}
	aria-pressed={pressed}
	onclick={click}
	onpointerenter={show}
	onpointerleave={leave}
	onfocus={focus}
	onblur={hide}
>
	<Icon name={icon} />
</svelte:element>
<!-- Repeats the button's name, so it is hidden from screen readers. -->
<span bind:this={hint} popover="manual" class="hint" aria-hidden="true" role="presentation" onpointerenter={() => clearTimeout(timer)} onpointerleave={leave}>{label}</span>
