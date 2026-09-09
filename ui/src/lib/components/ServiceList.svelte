<script lang="ts">
	import { Container, Trash2 } from 'lucide-svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { deleteService, type Service } from '$lib/apps';

	let {
		appId,
		services,
		onchanged
	}: {
		appId: string;
		services: Service[];
		onchanged: (services: Service[]) => void;
	} = $props();

	let deletingId = $state<string | null>(null);
	let error = $state('');

	async function remove(id: string) {
		if (deletingId) return;
		deletingId = id;
		error = '';
		try {
			await deleteService(appId, id);
			onchanged(services.filter((s) => s.id !== id));
		} catch (err) {
			error = err instanceof Error ? err.message : 'Could not delete service.';
		} finally {
			deletingId = null;
		}
	}
</script>

<div class="grid gap-2">
	{#if services.length === 0}
		<p
			class="flex items-center gap-2 rounded-lg border border-dashed px-3 py-2.5 text-sm text-zinc-500"
		>
			<Container class="size-4" />
			No services yet. Add your first container below.
		</p>
	{:else}
		<ul class="grid gap-1.5">
			{#each services as service (service.id)}
				<li class="flex items-center gap-2.5 rounded-lg border px-3 py-2 text-sm">
					<span
						class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-zinc-100 dark:bg-zinc-800"
					>
						<Container class="size-4" />
					</span>
					<span class="min-w-0 flex-1">
						<span class="block truncate font-medium">{service.name}</span>
						{#if service.image}
							<span class="block truncate font-mono text-xs text-zinc-500">{service.image}</span>
						{/if}
					</span>
					<Button
						variant="ghost"
						size="icon-sm"
						onclick={() => remove(service.id)}
						loading={deletingId === service.id}
						aria-label={`Delete service ${service.name}`}
					>
						<Trash2 class="text-red-500" />
					</Button>
				</li>
			{/each}
		</ul>
	{/if}
	{#if error}
		<p class="text-xs text-red-600 dark:text-red-400">{error}</p>
	{/if}
</div>
