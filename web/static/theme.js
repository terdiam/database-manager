// Loaded before first paint so the stored theme does not flash. A file, not an inline
// script, so the Content-Security-Policy needs no exception for it.
try {
	const scheme = localStorage.getItem('color-scheme');
	if (scheme) document.querySelector('meta[name="color-scheme"]').content = scheme;
	const accent = localStorage.getItem('accent');
	if (accent) document.documentElement.dataset.accent = accent;
} catch {}
