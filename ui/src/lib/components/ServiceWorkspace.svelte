<script lang="ts">
	import { tick } from 'svelte';
	import { CircleAlert, LocateFixed, Minus, Plus, Workflow } from 'lucide-svelte';
	import {
		getServiceState,
		listServices,
		updateServicePosition,
		type Service,
		type ServiceState
	} from '$lib/api/services';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import ServiceCard from './ServiceCard.svelte';
	import ServiceCreateDialog from './ServiceCreateDialog.svelte';
	import ServiceDetailPanel from './ServiceDetailPanel.svelte';

	let { appId }: { appId: string } = $props();

	const CARD_WIDTH = 272;
	const CARD_HEIGHT = 132;
	const MIN_ZOOM = 0.5;
	const MAX_ZOOM = 1.5;

	let services = $state<Service[]>([]);
	let status = $state<'loading' | 'ready' | 'error'>('loading');
	let error = $state('');
	let actionError = $state('');
	let selectedId = $state<string | null>(null);
	let createOpen = $state(false);
	let createPosition = $state({ x: 0, y: 0 });
	let canvasWidth = $state(0);
	let canvasHeight = $state(0);
	let zoom = $state(1);
	let pan = $state({ x: 0, y: 0 });
	let statusOverrides = $state<Record<string, ServiceState>>({});
	let panDrag = $state<{
		pointerId: number;
		x: number;
		y: number;
		panX: number;
		panY: number;
	} | null>(null);

	const selectedService = $derived(services.find((service) => service.id === selectedId) ?? null);

	async function load(showLoading = true) {
		if (showLoading) status = 'loading';
		error = '';
		try {
			services = await listServices(appId);
			status = 'ready';
			if (selectedId && !services.some((service) => service.id === selectedId)) selectedId = null;
			if (showLoading) {
				await tick();
				centerContent();
			}
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not load services.';
			if (showLoading) status = 'error';
			throw caught;
		}
	}

	$effect(() => {
		if (appId) load().catch(() => undefined);
	});

	function centerContent() {
		zoom = 1;
		if (!services.length || !canvasWidth || !canvasHeight) {
			pan = { x: 0, y: 0 };
			return;
		}
		const minX = Math.min(...services.map((service) => service.position_x));
		const maxX = Math.max(...services.map((service) => service.position_x + CARD_WIDTH));
		const minY = Math.min(...services.map((service) => service.position_y));
		const maxY = Math.max(...services.map((service) => service.position_y + CARD_HEIGHT));
		pan = {
			x: Math.round(canvasWidth / 2 - (minX + maxX) / 2),
			y: Math.round(canvasHeight / 2 - (minY + maxY) / 2)
		};
	}

	function setZoom(next: number) {
		const clamped = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(next * 10) / 10));
		if (clamped === zoom) return;
		const centerX = canvasWidth / 2;
		const centerY = canvasHeight / 2;
		const worldCenterX = (centerX - pan.x) / zoom;
		const worldCenterY = (centerY - pan.y) / zoom;
		pan = {
			x: Math.round(centerX - worldCenterX * clamped),
			y: Math.round(centerY - worldCenterY * clamped)
		};
		zoom = clamped;
	}

	function canvasPointerDown(event: PointerEvent) {
		if (event.button !== 0) return;
		if (
			event.target instanceof Element &&
			event.target.closest('[data-service-card], [data-controls]')
		)
			return;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		panDrag = {
			pointerId: event.pointerId,
			x: event.clientX,
			y: event.clientY,
			panX: pan.x,
			panY: pan.y
		};
	}

	function canvasPointerMove(event: PointerEvent) {
		if (!panDrag || event.pointerId !== panDrag.pointerId) return;
		pan = {
			x: Math.round(panDrag.panX + event.clientX - panDrag.x),
			y: Math.round(panDrag.panY + event.clientY - panDrag.y)
		};
	}

	function canvasPointerUp(event: PointerEvent) {
		if (!panDrag || event.pointerId !== panDrag.pointerId) return;
		(event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
		panDrag = null;
	}

	function previewPosition(serviceId: string, x: number, y: number) {
		services = services.map((service) =>
			service.id === serviceId
				? { ...service, position_x: Math.round(x), position_y: Math.round(y) }
				: service
		);
	}

	async function persistPosition(
		serviceId: string,
		previousX: number,
		previousY: number,
		x: number,
		y: number
	) {
		const positionX = Math.round(x);
		const positionY = Math.round(y);
		actionError = '';
		try {
			const updated = await updateServicePosition(appId, serviceId, positionX, positionY);
			if (updated) updateLocalService(updated);
		} catch (caught) {
			previewPosition(serviceId, previousX, previousY);
			actionError =
				caught instanceof Error ? caught.message : 'Could not save the service position.';
		}
	}

	function updateLocalService(updated: Service) {
		services = services.map((service) => (service.id === updated.id ? updated : service));
	}

	function setBusy(serviceId: string, next: ServiceState | null) {
		if (next) statusOverrides = { ...statusOverrides, [serviceId]: next };
		else {
			const remaining = { ...statusOverrides };
			delete remaining[serviceId];
			statusOverrides = remaining;
		}
	}

	function openCreate() {
		const panelWidth = selectedId && canvasWidth >= 640 ? 460 : 0;
		const visibleCenterX = (canvasWidth - panelWidth) / 2;
		const visibleCenterY = canvasHeight / 2;
		const stagger = (services.length % 6) * 26;
		createPosition = {
			x: Math.round((visibleCenterX - pan.x) / zoom - CARD_WIDTH / 2 + stagger),
			y: Math.round((visibleCenterY - pan.y) / zoom - CARD_HEIGHT / 2 + stagger)
		};
		createOpen = true;
	}

	function handleCreated(service: Service) {
		services = [...services, service];
		selectedId = service.id;
		createOpen = false;
	}

	function handleDeleted(serviceId: string) {
		services = services.filter((service) => service.id !== serviceId);
		selectedId = null;
		setBusy(serviceId, null);
	}
</script>

<section
	bind:clientWidth={canvasWidth}
	bind:clientHeight={canvasHeight}
	class:cursor-grabbing={panDrag}
	class="relative min-h-0 flex-1 touch-none cursor-grab overflow-hidden bg-[#0d0d0f] text-zinc-100 outline-none"
	aria-label="Service workspace canvas"
	onpointerdown={canvasPointerDown}
	onpointermove={canvasPointerMove}
	onpointerup={canvasPointerUp}
	onpointercancel={canvasPointerUp}
>
	<div
		class="pointer-events-none absolute inset-0 opacity-60"
		style={`background-image: radial-gradient(circle, rgba(255,255,255,0.16) 1px, transparent 1px); background-position: ${pan.x}px ${pan.y}px; background-size: ${24 * zoom}px ${24 * zoom}px;`}
	></div>

	{#if status === 'loading'}
		<div class="absolute inset-0 z-20 grid place-items-center bg-[#0d0d0f]/80">
			<div class="grid justify-items-center gap-3 text-sm text-zinc-500">
				<Spinner class="size-6 text-violet-300" />
				Loading services…
			</div>
		</div>
	{:else if status === 'error'}
		<div class="absolute inset-0 z-20 grid place-items-center p-6">
			<div
				class="grid max-w-sm justify-items-center gap-3 rounded-xl border border-white/10 bg-zinc-900 p-6 text-center shadow-xl"
			>
				<CircleAlert class="size-6 text-red-300" aria-hidden="true" />
				<p class="text-sm text-zinc-300">{error}</p>
				<Button
					variant="outline"
					class="border-white/10 bg-white/5 text-zinc-100"
					onclick={() => load().catch(() => undefined)}
				>
					Try again
				</Button>
			</div>
		</div>
	{:else}
		<div
			class="absolute left-0 top-0 origin-top-left"
			style={`transform: translate(${pan.x}px, ${pan.y}px) scale(${zoom})`}
		>
			{#each services as service (service.id)}
				<ServiceCard
					{service}
					status={getServiceState(service, statusOverrides[service.id])}
					selected={selectedId === service.id}
					{zoom}
					onselect={() => (selectedId = service.id)}
					onpositionpreview={(x, y) => previewPosition(service.id, x, y)}
					onpositionend={(previousX, previousY, x, y) =>
						persistPosition(service.id, previousX, previousY, x, y)}
					onpositioncancel={(x, y) => previewPosition(service.id, x, y)}
				/>
			{/each}
		</div>

		{#if services.length === 0}
			<div class="pointer-events-none absolute inset-0 grid place-items-center p-6">
				<div class="grid max-w-sm justify-items-center gap-2 text-center">
					<div
						class="mb-2 flex size-11 items-center justify-center rounded-xl border border-white/8 bg-zinc-900 text-zinc-500"
					>
						<Workflow class="size-5" aria-hidden="true" />
					</div>
					<h2 class="text-sm font-semibold text-zinc-200">Your service workspace is empty</h2>
					<p class="text-xs leading-5 text-zinc-500">
						Add an empty service, then upload and deploy a package from its detail panel.
					</p>
				</div>
			</div>
		{/if}
	{/if}

	<div data-controls class="absolute left-4 top-4 z-30">
		<Button onclick={openCreate} disabled={status !== 'ready'} class="shadow-lg shadow-black/30">
			<Plus /> Add service
		</Button>
	</div>

	<div
		data-controls
		class="absolute bottom-4 left-4 z-30 flex items-center gap-1 rounded-lg border border-white/10 bg-zinc-900/95 p-1 shadow-xl shadow-black/30 backdrop-blur"
	>
		<Button
			variant="ghost"
			size="icon-sm"
			class="text-zinc-300 hover:bg-white/8"
			onclick={() => setZoom(zoom - 0.1)}
			disabled={zoom <= MIN_ZOOM}
			aria-label="Zoom out"
		>
			<Minus />
		</Button>
		<span class="w-11 text-center font-mono text-[11px] text-zinc-500"
			>{Math.round(zoom * 100)}%</span
		>
		<Button
			variant="ghost"
			size="icon-sm"
			class="text-zinc-300 hover:bg-white/8"
			onclick={() => setZoom(zoom + 0.1)}
			disabled={zoom >= MAX_ZOOM}
			aria-label="Zoom in"
		>
			<Plus />
		</Button>
		<div class="mx-0.5 h-4 w-px bg-white/10"></div>
		<Button
			variant="ghost"
			size="icon-sm"
			class="text-zinc-300 hover:bg-white/8"
			onclick={centerContent}
			aria-label="Reset canvas view"
			title="Reset view"
		>
			<LocateFixed />
		</Button>
	</div>

	{#if actionError}
		<div
			data-controls
			role="alert"
			class="absolute bottom-4 left-1/2 z-50 flex max-w-[calc(100%-2rem)] -translate-x-1/2 items-center gap-2 rounded-lg border border-red-500/25 bg-red-950/90 px-3 py-2 text-xs text-red-200 shadow-xl"
		>
			<CircleAlert class="size-3.5 shrink-0" aria-hidden="true" />
			<span class="truncate">{actionError}</span>
			<button
				class="ml-1 text-red-300 hover:text-white"
				onclick={() => (actionError = '')}
				aria-label="Dismiss error">×</button
			>
		</div>
	{/if}

	{#if selectedService}
		<div
			class="absolute inset-0 z-30 bg-black/25 sm:hidden"
			data-controls
			role="presentation"
			onclick={() => (selectedId = null)}
		></div>
		<ServiceDetailPanel
			{appId}
			service={selectedService}
			status={getServiceState(selectedService, statusOverrides[selectedService.id])}
			onclose={() => (selectedId = null)}
			onrefresh={() => load(false).catch(() => undefined)}
			onupdated={updateLocalService}
			ondeleted={handleDeleted}
			onbusychange={(next) => setBusy(selectedService.id, next)}
		/>
	{/if}
</section>

<ServiceCreateDialog
	open={createOpen}
	{appId}
	position={createPosition}
	onclose={() => (createOpen = false)}
	oncreated={handleCreated}
/>
