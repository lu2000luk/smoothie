<script lang="ts">
	import { ChevronDown, Pencil, Trash2 } from 'lucide-svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Card, CardPanel } from '$lib/components/ui/card/index.js';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import ServiceCreateForm from './ServiceCreateForm.svelte';
	import ServiceList from './ServiceList.svelte';
	import { listServices, type App, type Service } from '$lib/apps';
	import { cn } from '$lib/utils.js';

	let {
		app,
		onrename,
		ondelete
	}: {
		app: App;
		onrename: (app: App) => void;
		ondelete: (app: App) => void;
	} = $props();

	let expanded = $state(false);
	let services = $state<Service[]>([]);
	let loaded = $state(false);
	let loading = $state(false);

	async function toggle() {
		expanded = !expanded;
		if (expanded && !loaded && !loading) {
			loading = true;
			try {
				services = await listServices(app.id);
				loaded = true;
			} catch {
				services = [];
			} finally {
				loading = false;
			}
		}
	}
</script>

<Card class="w-full">
	<CardPanel class="flex items-center gap-3">
		<AppIconDisplay kind={app.icon_kind} value={app.icon} />
		<button
			type="button"
			onclick={toggle}
			class="min-w-0 flex-1 text-left"
			aria-expanded={expanded}
		>
			<span class="block truncate text-sm font-semibold">{app.name}</span>
			<span class="block truncate font-mono text-xs text-zinc-500">{app.id}</span>
		</button>
		<Button
			variant="ghost"
			size="icon-sm"
			onclick={() => onrename(app)}
			aria-label={`Rename ${app.name}`}
		>
			<Pencil />
		</Button>
		<Button
			variant="ghost"
			size="icon-sm"
			onclick={() => ondelete(app)}
			aria-label={`Delete ${app.name}`}
		>
			<Trash2 class="text-red-500" />
		</Button>
		<Button
			variant="ghost"
			size="icon-sm"
			onclick={toggle}
			aria-label={expanded ? 'Collapse' : 'Expand'}
		>
			<ChevronDown class={cn('transition-transform', expanded && 'rotate-180')} />
		</Button>
	</CardPanel>
	{#if expanded}
		<CardPanel class="grid gap-3 border-t pt-4">
			{#if loading}
				<p class="text-sm text-zinc-500">Loading services…</p>
			{:else}
				<ServiceList appId={app.id} {services} onchanged={(next) => (services = next)} />
				<ServiceCreateForm appId={app.id} oncreated={(s) => (services = [...services, s])} />
			{/if}
		</CardPanel>
	{/if}
</Card>
