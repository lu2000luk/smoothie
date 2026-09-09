<script lang="ts">
	import { Box, GripVertical } from 'lucide-svelte';
	import type { Service, ServiceState } from '$lib/api/services';
	import { cn } from '$lib/utils';
	import ServiceStatusBadge from './ServiceStatusBadge.svelte';

	let {
		service,
		status,
		selected,
		zoom,
		onselect,
		onpositionpreview,
		onpositionend,
		onpositioncancel
	}: {
		service: Service;
		status: ServiceState;
		selected: boolean;
		zoom: number;
		onselect: () => void;
		onpositionpreview: (x: number, y: number) => void;
		onpositionend: (previousX: number, previousY: number, x: number, y: number) => void;
		onpositioncancel: (x: number, y: number) => void;
	} = $props();

	let drag = $state<{
		pointerId: number;
		clientX: number;
		clientY: number;
		originX: number;
		originY: number;
		currentX: number;
		currentY: number;
		moved: boolean;
	} | null>(null);
	let suppressClick = false;

	function isInteractive(target: EventTarget | null): boolean {
		return (
			target instanceof Element && Boolean(target.closest('button, a, input, textarea, select'))
		);
	}

	function pointerDown(event: PointerEvent) {
		if (event.button !== 0 || isInteractive(event.target)) return;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		drag = {
			pointerId: event.pointerId,
			clientX: event.clientX,
			clientY: event.clientY,
			originX: service.position_x,
			originY: service.position_y,
			currentX: service.position_x,
			currentY: service.position_y,
			moved: false
		};
	}

	function pointerMove(event: PointerEvent) {
		if (!drag || event.pointerId !== drag.pointerId) return;
		const deltaX = event.clientX - drag.clientX;
		const deltaY = event.clientY - drag.clientY;
		if (!drag.moved && Math.hypot(deltaX, deltaY) < 4) return;
		drag.moved = true;
		drag.currentX = Math.round(drag.originX + deltaX / zoom);
		drag.currentY = Math.round(drag.originY + deltaY / zoom);
		onpositionpreview(drag.currentX, drag.currentY);
	}

	function pointerUp(event: PointerEvent) {
		if (!drag || event.pointerId !== drag.pointerId) return;
		(event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
		if (drag.moved) {
			suppressClick = true;
			onpositionend(drag.originX, drag.originY, drag.currentX, drag.currentY);
		}
		drag = null;
	}

	function pointerCancel(event: PointerEvent) {
		if (!drag || event.pointerId !== drag.pointerId) return;
		onpositioncancel(drag.originX, drag.originY);
		drag = null;
	}

	function click() {
		if (suppressClick) {
			suppressClick = false;
			return;
		}
		onselect();
	}

	function keydown(event: KeyboardEvent) {
		if (event.key !== 'Enter' && event.key !== ' ') return;
		event.preventDefault();
		onselect();
	}
</script>

<div
	data-service-card
	role="button"
	tabindex="0"
	aria-label={`Open ${service.name} service`}
	aria-pressed={selected}
	class={cn(
		'group absolute w-68 touch-none cursor-grab select-none rounded-xl border border-white/10 bg-zinc-900/95 p-4 text-left shadow-xl shadow-black/25 ring-4 ring-transparent backdrop-blur transition-[border-color,box-shadow] outline-none hover:border-white/20 focus-visible:border-violet-400 focus-visible:ring-violet-400/20',
		selected && 'border-violet-400 shadow-violet-950/40 ring-violet-400/20',
		drag?.moved && 'cursor-grabbing'
	)}
	style={`transform: translate(${service.position_x}px, ${service.position_y}px)`}
	onpointerdown={pointerDown}
	onpointermove={pointerMove}
	onpointerup={pointerUp}
	onpointercancel={pointerCancel}
	onclick={click}
	onkeydown={keydown}
>
	<div class="flex items-start gap-3">
		<div
			class="flex size-10 shrink-0 items-center justify-center rounded-lg border border-white/8 bg-zinc-800 text-zinc-300 shadow-inner"
		>
			<Box class="size-5" aria-hidden="true" />
		</div>
		<div class="min-w-0 flex-1">
			<p class="truncate text-sm font-semibold text-zinc-50">{service.name}</p>
			<div class="mt-1">
				<ServiceStatusBadge {status} />
			</div>
		</div>
		<GripVertical
			class="size-4 shrink-0 text-zinc-600 transition-colors group-hover:text-zinc-400"
			aria-hidden="true"
		/>
	</div>
	<div
		class="mt-4 flex items-center justify-between border-t border-white/6 pt-3 text-[11px] text-zinc-500"
	>
		<span>{service.container_port ? `Port ${service.container_port}` : 'Port not set'}</span>
		<span>{service.package_ids.length} package{service.package_ids.length === 1 ? '' : 's'}</span>
	</div>
</div>
