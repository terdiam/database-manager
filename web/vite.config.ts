import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	// Dev only: the Rust server answers /api; in production it also serves this build.
	server: { proxy: { '/api': 'http://127.0.0.1:8080' } },
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Written into the page as a <meta> tag, with hashes for SvelteKit's own inline script.
			// frame-ancestors cannot go in a meta tag: the server sends it as a header.
			csp: {
				mode: 'hash',
				directives: {
					'default-src': ['self'],
					'script-src': ['self', 'https://challenges.cloudflare.com'],
					'frame-src': ['https://challenges.cloudflare.com'],
					'style-src': ['self'],
					// style="" attributes in markup (SvelteKit's wrapper and route announcer).
					'style-src-attr': ['unsafe-inline'],
					'img-src': ['self', 'data:'],
					'connect-src': ['self'],
					'object-src': ['none'],
					'base-uri': ['self'],
					'form-action': ['self']
				}
			},
			// SPA: the Rust server embeds this build and serves the fallback for unknown paths.
			adapter: adapter({ fallback: '200.html', precompress: true })
		})
	]
});
