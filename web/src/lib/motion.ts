// Every animation library is loaded on demand, so none of them weighs on the first paint,
// and none is downloaded at all when the user asks for reduced motion.
export const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

let gsapLoad: ReturnType<typeof importGsap> | undefined;
async function importGsap() {
	const [{ gsap }, { ScrollTrigger }, { SplitText }] = await Promise.all([
		import('gsap'),
		import('gsap/ScrollTrigger'),
		import('gsap/SplitText')
	]);
	gsap.registerPlugin(ScrollTrigger, SplitText);
	return { gsap, SplitText };
}
export const loadGsap = () => (gsapLoad ??= importGsap());

/** Attachment: the element's text appears word by word once it scrolls into view. */
export function revealWords(text: () => string) {
	return (node: Element) => {
		const element = node as HTMLElement;
		// The attachment owns the text node, because SplitText rewrites it.
		element.textContent = text();
		if (reducedMotion()) return;

		let cancelled = false;
		let undo = () => {};
		element.style.visibility = 'hidden';
		loadGsap().then(
			({ gsap, SplitText }) => {
				element.style.visibility = '';
				if (cancelled) return;
				const split = SplitText.create(element, { type: 'words' });
				const tween = gsap.from(split.words, {
					yPercent: 70,
					opacity: 0,
					duration: 0.5,
					stagger: 0.08,
					ease: 'power3.out',
					scrollTrigger: { trigger: element, start: 'top 95%', once: true }
				});
				undo = () => {
					tween.scrollTrigger?.kill();
					tween.kill();
					split.revert();
				};
			},
			() => (element.style.visibility = '')
		);
		return () => {
			cancelled = true;
			undo();
		};
	};
}

/** Attachment for a page container: cards slide in when first seen and lift on hover. */
export function cardMotion(root: Element) {
	if (reducedMotion()) return;
	let observer: MutationObserver | undefined;
	let cancelled = false;

	import('motion').then(({ animate, hover, inView }) => {
		if (cancelled) return;
		const seen = new WeakSet<Element>();

		const setup = (card: HTMLElement, order: number) => {
			if (seen.has(card)) return;
			seen.add(card);
			card.style.opacity = '0';
			// Default threshold (any pixel): a ratio would never be met by a very tall card.
			// Returning nothing makes inView stop observing after the first time.
			inView(card, () => {
				animate(card, { opacity: [0, 1], transform: ['translateY(16px)', 'translateY(0px)'] }, { duration: 0.45, delay: Math.min(order * 0.06, 0.36), ease: 'easeOut' }).then(() => {
					card.style.removeProperty('opacity');
					card.style.removeProperty('transform');
				});
			});

			// No lift on forms and tables: controls must not move under the pointer.
			if (card.matches('form, .table-wrap, :has(form)')) return;
			card.classList.add('lift');
			hover(card, () => {
				animate(card, { transform: 'translateY(-3px)' }, { duration: 0.18 });
				return () => animate(card, { transform: 'translateY(0px)' }, { duration: 0.18 });
			});
		};

		const scan = () => root.querySelectorAll<HTMLElement>('.card').forEach((card, i) => setup(card, i));
		scan();
		observer = new MutationObserver((changes) => {
			if (changes.some((c) => [...c.addedNodes].some((n) => n instanceof Element))) scan();
		});
		observer.observe(root, { childList: true, subtree: true });
	});

	return () => {
		cancelled = true;
		observer?.disconnect();
	};
}

/** Attachment: a number counts up to its value. */
export function countUp(value: () => number) {
	return (node: Element) => {
		const target = value();
		node.textContent = String(target);
		if (reducedMotion() || !target) return;
		let stop = () => {};
		let cancelled = false;
		import('motion').then(({ animate }) => {
			if (cancelled) return;
			const controls = animate(0, target, { duration: 0.8, ease: 'easeOut', onUpdate: (n) => (node.textContent = String(Math.round(n))) });
			stop = () => controls.stop();
		});
		return () => {
			cancelled = true;
			stop();
		};
	};
}

/** Smooth wheel scrolling. Dialogs, popovers and pickers keep their native scroll. */
export function smoothScroll(): () => void {
	if (reducedMotion()) return () => {};
	let destroy = () => {};
	let cancelled = false;
	Promise.all([import('lenis'), import('lenis/dist/lenis.css')]).then(([{ default: Lenis }]) => {
		if (cancelled) return;
		const lenis = new Lenis({
			autoRaf: true,
			anchors: true,
			allowNestedScroll: true,
			stopInertiaOnNavigate: true,
			prevent: (node) => !!node.closest?.('dialog, [popover], select')
		});
		destroy = () => lenis.destroy();
	});
	return () => {
		cancelled = true;
		destroy();
	};
}

/** Attachment: descendants marked `data-enter` rise in one after another. */
export function enter(root: Element) {
	if (reducedMotion()) return;
	const items = [...root.querySelectorAll<HTMLElement>('[data-enter]')];
	items.forEach((item) => (item.style.opacity = '0'));
	const show = () => items.forEach((item) => item.style.removeProperty('opacity'));
	import('motion').then(({ animate, stagger }) => {
		animate(items, { opacity: [0, 1], transform: ['translateY(18px)', 'translateY(0px)'] }, { duration: 0.5, delay: stagger(0.07), ease: 'easeOut' }).then(() => {
			show();
			items.forEach((item) => item.style.removeProperty('transform'));
		});
	}, show);
}

/** Attachment: a looping compositor-only animation, stopped when the element goes away. */
export function loop(keyframes: Record<string, (string | number)[]>, duration: number, delay = 0) {
	return (node: Element) => {
		if (reducedMotion()) return;
		let stop = () => {};
		let cancelled = false;
		import('motion').then(({ animate }) => {
			if (cancelled) return;
			const controls = animate(node, keyframes, { duration, delay, repeat: Infinity, ease: 'easeInOut' });
			stop = () => controls.stop();
		});
		return () => {
			cancelled = true;
			stop();
		};
	};
}

/** A short side-to-side shake to point at a rejected form. */
export function shake(element: Element) {
	if (reducedMotion()) return;
	import('motion').then(({ animate }) => animate(element, { transform: [0, -10, 9, -6, 4, 0].map((x) => `translateX(${x}px)`) }, { duration: 0.4 }));
}
