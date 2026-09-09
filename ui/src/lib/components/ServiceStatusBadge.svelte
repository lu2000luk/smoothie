<script lang="ts">
	import { Circle, CircleAlert, LoaderCircle, PackageOpen, Square } from 'lucide-svelte';
	import type { ServiceState } from '$lib/api/services';

	let { status }: { status: ServiceState } = $props();

	const label = $derived(
		status === 'no-package' ? 'No package' : status.charAt(0).toUpperCase() + status.slice(1)
	);
</script>

<span
	class:text-zinc-400={status === 'no-package' || status === 'stopped'}
	class:text-violet-300={status === 'uploading' || status === 'deploying'}
	class:text-emerald-300={status === 'running'}
	class:text-red-300={status === 'failed'}
	class="inline-flex items-center gap-1.5 text-xs font-medium"
>
	{#if status === 'uploading' || status === 'deploying'}
		<LoaderCircle class="size-3 animate-spin" aria-hidden="true" />
	{:else if status === 'running'}
		<Circle class="size-2.5 fill-current" aria-hidden="true" />
	{:else if status === 'failed'}
		<CircleAlert class="size-3" aria-hidden="true" />
	{:else if status === 'no-package'}
		<PackageOpen class="size-3" aria-hidden="true" />
	{:else}
		<Square class="size-2.5 fill-current" aria-hidden="true" />
	{/if}
	{label}
</span>
