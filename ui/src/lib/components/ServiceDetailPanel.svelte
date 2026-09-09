<script lang="ts">
	import { Box, Rocket, Settings, X } from 'lucide-svelte';
	import {
		listServicePackages,
		type Service,
		type ServicePackage,
		type ServiceState
	} from '$lib/api/services';
	import { Button } from '$lib/components/ui/button/index.js';
	import ServiceDeploymentsTab from './ServiceDeploymentsTab.svelte';
	import ServiceSettingsTab from './ServiceSettingsTab.svelte';
	import ServiceStatusBadge from './ServiceStatusBadge.svelte';

	let {
		appId,
		service,
		status,
		onclose,
		onrefresh,
		onupdated,
		ondeleted,
		onbusychange
	}: {
		appId: string;
		service: Service;
		status: ServiceState;
		onclose: () => void;
		onrefresh: () => Promise<void>;
		onupdated: (service: Service) => void;
		ondeleted: (serviceId: string) => void;
		onbusychange: (status: ServiceState | null) => void;
	} = $props();

	let tab = $state<'deployments' | 'settings'>('deployments');
	let packages = $state<ServicePackage[]>([]);
	let packagesLoading = $state(true);
	let packagesError = $state('');
	let loadSequence = 0;

	async function loadPackages() {
		const sequence = ++loadSequence;
		packagesLoading = true;
		packagesError = '';
		try {
			const result = await listServicePackages(appId, service.id);
			if (sequence === loadSequence) packages = result;
		} catch (caught) {
			if (sequence === loadSequence) {
				packagesError = caught instanceof Error ? caught.message : 'Could not load packages.';
			}
		} finally {
			if (sequence === loadSequence) packagesLoading = false;
		}
	}

	$effect(() => {
		service.id;
		tab = 'deployments';
		packages = [];
		loadPackages();
	});

	function handleKey(event: KeyboardEvent) {
		if (event.key === 'Escape') onclose();
	}
</script>

<svelte:window onkeydown={handleKey} />

<aside
	data-controls
	class="absolute right-0 top-0 z-40 flex h-full w-full max-w-115 flex-col border-l border-white/10 bg-zinc-900/98 text-zinc-100 shadow-2xl shadow-black/45 backdrop-blur-xl max-sm:top-auto max-sm:bottom-0 max-sm:h-[78%] max-sm:max-w-none max-sm:rounded-t-2xl max-sm:border-t max-sm:border-l-0"
	aria-label={`${service.name} details`}
>
	<header class="flex items-start gap-3 border-b border-white/8 px-5 py-4">
		<div
			class="flex size-10 shrink-0 items-center justify-center rounded-lg border border-white/8 bg-zinc-800 text-zinc-300"
		>
			<Box class="size-5" aria-hidden="true" />
		</div>
		<div class="min-w-0 flex-1">
			<h2 class="truncate text-sm font-semibold">{service.name}</h2>
			<div class="mt-1"><ServiceStatusBadge {status} /></div>
		</div>
		<Button
			variant="ghost"
			size="icon-sm"
			class="text-zinc-300 hover:bg-white/8"
			onclick={onclose}
			aria-label="Close service details"
		>
			<X />
		</Button>
	</header>

	<div
		class="flex gap-1 border-b border-white/8 px-4 pt-2"
		role="tablist"
		aria-label="Service details"
	>
		<button
			type="button"
			role="tab"
			aria-selected={tab === 'deployments'}
			aria-controls="service-deployments-panel"
			class:border-violet-400={tab === 'deployments'}
			class:text-zinc-100={tab === 'deployments'}
			class="flex items-center gap-1.5 border-b-2 border-transparent px-3 py-2.5 text-xs font-medium text-zinc-500 outline-none hover:text-zinc-300 focus-visible:ring-2 focus-visible:ring-violet-400/40"
			onclick={() => (tab = 'deployments')}
		>
			<Rocket class="size-3.5" aria-hidden="true" /> Deployments
		</button>
		<button
			type="button"
			role="tab"
			aria-selected={tab === 'settings'}
			aria-controls="service-settings-panel"
			class:border-violet-400={tab === 'settings'}
			class:text-zinc-100={tab === 'settings'}
			class="flex items-center gap-1.5 border-b-2 border-transparent px-3 py-2.5 text-xs font-medium text-zinc-500 outline-none hover:text-zinc-300 focus-visible:ring-2 focus-visible:ring-violet-400/40"
			onclick={() => (tab = 'settings')}
		>
			<Settings class="size-3.5" aria-hidden="true" /> Settings
		</button>
	</div>

	<div class="min-h-0 flex-1 overflow-y-auto px-5 py-5">
		{#if tab === 'deployments'}
			<div id="service-deployments-panel" role="tabpanel">
				<ServiceDeploymentsTab
					{appId}
					{service}
					{status}
					{packages}
					{packagesLoading}
					{packagesError}
					onreloadpackages={loadPackages}
					{onrefresh}
					onserviceupdated={onupdated}
					{onbusychange}
				/>
			</div>
		{:else}
			<div id="service-settings-panel" role="tabpanel">
				<ServiceSettingsTab {appId} {service} {packages} {onupdated} {ondeleted} />
			</div>
		{/if}
	</div>
</aside>
