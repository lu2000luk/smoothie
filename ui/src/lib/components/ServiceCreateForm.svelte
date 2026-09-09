<script lang="ts">
	import { Plus } from 'lucide-svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { createService, type Service } from '$lib/apps';

	let {
		appId,
		oncreated
	}: {
		appId: string;
		oncreated: (service: Service) => void;
	} = $props();

	let name = $state('');
	let image = $state('');
	let saving = $state(false);
	let error = $state('');

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!name.trim() || saving) return;
		saving = true;
		error = '';
		try {
			const service = await createService(appId, name.trim(), image.trim() || null);
			name = '';
			image = '';
			oncreated(service);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Could not create service.';
		} finally {
			saving = false;
		}
	}
</script>

<form class="flex flex-col gap-2 sm:flex-row" onsubmit={submit}>
	<input
		bind:value={name}
		placeholder="Service name"
		maxlength={64}
		autocomplete="off"
		aria-label="Service name"
		class="h-9 flex-1 rounded-lg border border-zinc-300 bg-white px-3 text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/30 dark:border-zinc-700 dark:bg-zinc-950"
	/>
	<input
		bind:value={image}
		placeholder="Image (e.g. nginx:latest)"
		maxlength={256}
		autocomplete="off"
		aria-label="Container image"
		class="h-9 flex-1 rounded-lg border border-zinc-300 bg-white px-3 font-mono text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/30 dark:border-zinc-700 dark:bg-zinc-950"
	/>
	<Button type="submit" loading={saving} disabled={!name.trim()}>
		<Plus />
		<span>Add</span>
	</Button>
</form>
{#if error}
	<p class="mt-1.5 text-xs text-red-600 dark:text-red-400">{error}</p>
{/if}
