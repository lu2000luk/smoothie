<script lang="ts">
	import {
		Bell,
		Box,
		Cloud,
		Cpu,
		Database,
		Flag,
		Folder,
		Globe,
		Heart,
		Image,
		Key,
		Layers,
		Lock,
		Monitor,
		Music,
		Package,
		Palette,
		Rocket,
		Server,
		Settings,
		Shield,
		Smartphone,
		Star,
		Tag,
		Terminal,
		Video,
		Wifi,
		Wrench,
		Zap,
		PackageOpen
	} from 'lucide-svelte';
	import type { IconKind } from '$lib/apps';
	import { cn } from '$lib/utils.js';

	let {
		kind,
		value,
		size = 'md',
		class: className
	}: {
		kind: IconKind | null | undefined;
		value: string | null | undefined;
		size?: 'sm' | 'md' | 'lg';
		class?: string;
	} = $props();

	const sizeClass = $derived(
		size === 'sm' ? 'size-9 text-lg' : size === 'lg' ? 'size-14 text-3xl' : 'size-11 text-2xl'
	);

	const iconComponents: Record<string, typeof Box> = {
		box: Box,
		server: Server,
		database: Database,
		globe: Globe,
		cloud: Cloud,
		cpu: Cpu,
		zap: Zap,
		rocket: Rocket,
		layers: Layers,
		package: Package,
		terminal: Terminal,
		monitor: Monitor,
		smartphone: Smartphone,
		wifi: Wifi,
		shield: Shield,
		lock: Lock,
		key: Key,
		bell: Bell,
		star: Star,
		heart: Heart,
		flag: Flag,
		tag: Tag,
		settings: Settings,
		wrench: Wrench,
		palette: Palette,
		image: Image,
		music: Music,
		video: Video,
		folder: Folder
	};

	const ActiveIcon = $derived(kind === 'icon' && value ? (iconComponents[value] ?? Box) : null);
	const fallbackLetter = $derived('A');
</script>

<span
	class={cn(
		'bg-muted text-foreground inline-flex shrink-0 items-center justify-center overflow-hidden rounded-xl font-semibold',
		sizeClass,
		className
	)}
>
	{#if kind === 'emoji' && value}
		<span aria-hidden="true">{value}</span>
	{:else if kind === 'upload' && value}
		<img src={value} alt="" class="size-full object-cover" />
	{:else if ActiveIcon}
		<ActiveIcon class="size-1/2" />
	{:else if kind === 'icon' && value}
		<span aria-hidden="true" class="text-lg">📦</span>
	{:else}
		<span aria-hidden="true"><PackageOpen class="size-1/2" /></span>
		<span class="sr-only">{fallbackLetter}</span>
	{/if}
</span>
