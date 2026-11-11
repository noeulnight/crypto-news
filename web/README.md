# crypto-news web

SvelteKit 3 (Svelte 5 runes, TypeScript) client-side SPA for the crypto-news dashboard.
The Rust backend serves the JSON API on `http://127.0.0.1:8787` and, in production, the static files in `build/`.

Needs Node 22.17 or newer; `.nvmrc` pins the major version used here (`nvm use`).

```sh
pnpm install
pnpm dev      # http://localhost:5173, proxies /api (incl. SSE /api/stream) to 127.0.0.1:8787
pnpm check    # svelte-check
pnpm build    # static SPA into build/ (fallback: index.html)
```
