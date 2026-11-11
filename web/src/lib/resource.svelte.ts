import { untrack } from 'svelte';
import { errorMessage } from './api';

/** One async request slot with reactive data / error / loading, safe against out-of-order responses. */
export class Resource<T> {
	data = $state.raw<T | null>(null);
	error = $state<string | null>(null);
	loading = $state(false);
	#seq = 0;
	#fetcher: () => Promise<T>;

	constructor(fetcher: () => Promise<T>) {
		this.#fetcher = fetcher;
	}

	/** `reset` drops current data first (use when the request parameters changed). */
	load = async (reset = false): Promise<void> => {
		const seq = ++this.#seq;
		if (reset) this.data = null;
		this.loading = true;
		try {
			const data = await this.#fetcher();
			if (seq !== this.#seq) return;
			this.data = data;
			this.error = null;
		} catch (e) {
			if (seq !== this.#seq) return;
			this.error = errorMessage(e);
		} finally {
			if (seq === this.#seq) this.loading = false;
		}
	};
}

/** Runs `run` on mount and again whenever what `deps` reads changes. Call during component init. */
export function loadOn(deps: () => unknown, run: () => void): void {
	$effect(() => {
		deps();
		untrack(run);
	});
}
