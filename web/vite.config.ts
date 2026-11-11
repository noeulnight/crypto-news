import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

const API_ORIGIN = 'http://127.0.0.1:8787';

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Runes mode for our own components, not for dependencies (as in the SvelteKit 3 template).
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Client-side SPA: the Rust server serves `build/` and falls back to index.html.
			adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' })
		})
	],
	server: {
		port: 5173,
		strictPort: true,
		// The proxy streams responses, so the SSE route /api/stream passes through unbuffered.
		proxy: {
			'/api': { target: API_ORIGIN, changeOrigin: true }
		}
	}
});
