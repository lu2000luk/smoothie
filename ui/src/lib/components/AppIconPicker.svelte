<script lang="ts">
	import {
		Check,
		Clock,
		Flag,
		Hand,
		LayoutGrid,
		Leaf,
		Lightbulb,
		Pencil,
		Plane,
		Plus,
		Search,
		Shuffle,
		Smile,
		Trophy,
		Upload,
		X
	} from 'lucide-svelte';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		ALL_EMOJIS,
		EMOJI_CATEGORIES,
		ICON_NAMES,
		getRecentIcons,
		pushRecentIcon,
		randomEmoji,
		type IconSelection
	} from '$lib/appIcons';
	import type { IconKind } from '$lib/apps';
	import { cn } from '$lib/utils.js';

	let {
		value,
		onselect,
		onclose
	}: {
		value: IconSelection;
		onselect: (sel: IconSelection) => void;
		onclose?: () => void;
	} = $props();

	let tab: 'emoji' | 'icons' | 'upload' = $state(
		value.kind === 'icon' ? 'icons' : value.kind === 'upload' ? 'upload' : 'emoji'
	);
	let query = $state('');
	let activeCategory = $state('people');
	let recent = $state<IconSelection[]>(getRecentIcons());
	let uploadPreview = $state<string | null>(value.kind === 'upload' ? (value.value ?? null) : null);
	let uploadError = $state('');
	let fileInput: HTMLInputElement | null = $state(null);

	const categoryIcons = [
		Clock,
		Smile,
		Leaf,
		Pencil,
		Trophy,
		Plane,
		Lightbulb,
		Check,
		Flag,
		LayoutGrid,
		Plus
	];
	const categoryIds = ['recent', ...EMOJI_CATEGORIES.map((c) => c.id)];

	function pick(sel: IconSelection) {
		recent = pushRecentIcon(sel);
		onselect(sel);
	}

	function pickEmoji(emoji: string) {
		pick({ kind: 'emoji', value: emoji });
	}

	function pickIconName(name: string) {
		pick({ kind: 'icon', value: name });
	}

	function handleRemove() {
		onselect({ kind: null, value: null });
		onclose?.();
	}

	function handleRandom() {
		const emoji = randomEmoji();
		pickEmoji(emoji);
	}

	const filteredEmojis = $derived.by(() => {
		const q = query.trim().toLowerCase();
		if (!q) return null;
		return ALL_EMOJIS.filter((e) => e.includes(query.trim())).slice(0, 120);
	});

	const visibleCategory = $derived.by(() => {
		if (query.trim()) return null;
		return EMOJI_CATEGORIES.find((c) => c.id === activeCategory) ?? EMOJI_CATEGORIES[0];
	});

	const filteredIcons = $derived.by(() => {
		const q = query.trim().toLowerCase();
		if (!q) return ICON_NAMES;
		return ICON_NAMES.filter((n) => n.toLowerCase().includes(q));
	});

	function handleFile(file: File | undefined) {
		uploadError = '';
		if (!file) return;
		if (!file.type.startsWith('image/')) {
			uploadError = 'Please choose an image file.';
			return;
		}
		if (file.size > 800_000) {
			uploadError = 'Image must be under 800 KB.';
			return;
		}
		const reader = new FileReader();
		reader.onload = () => {
			const url = String(reader.result ?? '');
			uploadPreview = url;
			pick({ kind: 'upload', value: url });
		};
		reader.readAsDataURL(file);
	}

	function iconLabel(name: string) {
		return name.replace(/-/g, ' ');
	}
</script>

<div
	class="w-[340px] overflow-hidden rounded-2xl border border-zinc-700 bg-zinc-900 text-zinc-100 shadow-2xl"
	role="dialog"
	aria-label="Choose app icon"
>
	<div class="flex items-center justify-between px-4 pt-3">
		<div class="flex items-center gap-4 text-sm font-medium">
			<button
				type="button"
				onclick={() => (tab = 'emoji')}
				class={cn(
					'pb-2 transition-colors',
					tab === 'emoji'
						? 'border-b-2 border-blue-500 text-zinc-50'
						: 'text-zinc-400 hover:text-zinc-200'
				)}
			>
				Emoji
			</button>
			<button
				type="button"
				onclick={() => (tab = 'icons')}
				class={cn(
					'pb-2 transition-colors',
					tab === 'icons'
						? 'border-b-2 border-blue-500 text-zinc-50'
						: 'text-zinc-400 hover:text-zinc-200'
				)}
			>
				Icons
			</button>
			<button
				type="button"
				onclick={() => (tab = 'upload')}
				class={cn(
					'pb-2 transition-colors',
					tab === 'upload'
						? 'border-b-2 border-blue-500 text-zinc-50'
						: 'text-zinc-400 hover:text-zinc-200'
				)}
			>
				Upload
			</button>
		</div>
		<button
			type="button"
			onclick={handleRemove}
			class="pb-2 text-sm text-zinc-400 transition-colors hover:text-zinc-100"
		>
			Remove
		</button>
	</div>

	<div class="flex items-center gap-2 px-3 pb-2">
		<label
			class="flex h-9 flex-1 items-center gap-2 rounded-lg border-2 border-blue-500 bg-zinc-800 px-2.5"
		>
			<Search class="size-4 shrink-0 text-zinc-400" />
			<input
				bind:value={query}
				placeholder="Filter..."
				class="w-full bg-transparent text-sm text-zinc-100 outline-none placeholder:text-zinc-500"
			/>
			{#if query}
				<button type="button" onclick={() => (query = '')} aria-label="Clear filter">
					<X class="size-3.5 text-zinc-500 hover:text-zinc-200" />
				</button>
			{/if}
		</label>
		<button
			type="button"
			onclick={handleRandom}
			title="Random"
			aria-label="Random emoji"
			class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-zinc-800 text-zinc-300 transition-colors hover:bg-zinc-700 hover:text-white"
		>
			<Shuffle class="size-4" />
		</button>
		<button
			type="button"
			title="Tone picker"
			aria-label="Tone picker"
			class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-zinc-800 text-xl transition-colors hover:bg-zinc-700"
		>
			<Hand class="size-4 text-amber-300" />
		</button>
	</div>

	{#if tab === 'emoji'}
		<div class="h-64 overflow-y-auto px-3 pb-2">
			{#if filteredEmojis}
				<p class="py-1.5 text-xs font-medium text-zinc-400">Search results</p>
				<div class="grid grid-cols-8 gap-0.5">
					{#each filteredEmojis as emoji (emoji)}
						<button
							type="button"
							onclick={() => pickEmoji(emoji)}
							class="flex size-9 items-center justify-center rounded-md text-xl transition-colors hover:bg-zinc-700"
						>
							{emoji}
						</button>
					{/each}
				</div>
			{:else}
				<p class="py-1.5 text-xs font-medium text-zinc-400">Recent</p>
				<div class="flex gap-0.5">
					{#each recent as item, i (`${item.kind}-${item.value}-${i}`)}
						{#if item.kind === 'emoji' && item.value}
							<button
								type="button"
								onclick={() => pick(item)}
								class="flex size-9 items-center justify-center rounded-md bg-zinc-800 text-xl transition-colors hover:bg-zinc-700"
							>
								{item.value}
							</button>
						{:else if item.value}
							<button
								type="button"
								onclick={() => pick(item)}
								class="flex size-9 items-center justify-center rounded-md bg-zinc-800 transition-colors hover:bg-zinc-700"
							>
								<AppIconDisplay
									kind={item.kind as IconKind}
									value={item.value}
									size="sm"
									class="bg-transparent"
								/>
							</button>
						{/if}
					{/each}
				</div>
				<p class="py-1.5 text-xs font-medium text-zinc-400">{visibleCategory?.label}</p>
				<div class="grid grid-cols-8 gap-0.5">
					{#each visibleCategory?.emojis ?? [] as emoji (emoji)}
						<button
							type="button"
							onclick={() => pickEmoji(emoji)}
							class="flex size-9 items-center justify-center rounded-md text-xl transition-colors hover:bg-zinc-700"
						>
							{emoji}
						</button>
					{/each}
				</div>
			{/if}
		</div>
		<div class="flex items-center gap-1 border-t border-zinc-800 bg-zinc-900 px-2 py-2">
			{#each categoryIcons as Icon, i (i)}
				<button
					type="button"
					onclick={() => {
						activeCategory = categoryIds[Math.min(i, categoryIds.length - 1)] ?? 'people';
						query = '';
					}}
					aria-label={categoryIds[i]}
					class={cn(
						'flex size-8 items-center justify-center rounded-md transition-colors',
						(i === 0 && !query && activeCategory === 'people') || (i === 1 && !query)
							? 'bg-zinc-800 text-zinc-100'
							: 'text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200'
					)}
				>
					<Icon class="size-4" />
				</button>
			{/each}
		</div>
	{:else if tab === 'icons'}
		<div class="h-64 overflow-y-auto px-3 pb-3">
			<p class="py-1.5 text-xs font-medium text-zinc-400">App icons</p>
			<div class="grid grid-cols-6 gap-1.5">
				{#each filteredIcons as name (name)}
					<button
						type="button"
						onclick={() => pickIconName(name)}
						title={iconLabel(name)}
						class={cn(
							'flex size-11 flex-col items-center justify-center gap-0.5 rounded-lg border transition-colors',
							value.kind === 'icon' && value.value === name
								? 'border-blue-500 bg-blue-500/15 text-zinc-50'
								: 'border-transparent bg-zinc-800 text-zinc-300 hover:bg-zinc-700 hover:text-white'
						)}
					>
						<AppIconDisplay kind="icon" value={name} size="sm" class="bg-transparent" />
					</button>
				{/each}
			</div>
			{#if filteredIcons.length === 0}
				<p class="py-6 text-center text-sm text-zinc-500">No icons match “{query}”.</p>
			{/if}
		</div>
	{:else}
		<div class="grid gap-3 px-4 py-4">
			{#if uploadPreview}
				<div class="flex items-center gap-3">
					<img
						src={uploadPreview}
						alt="Uploaded icon preview"
						class="size-14 rounded-xl border border-zinc-700 object-cover"
					/>
					<div class="grid gap-1">
						<p class="text-sm text-zinc-200">Custom image selected</p>
						<button
							type="button"
							onclick={handleRemove}
							class="w-fit text-xs text-zinc-400 underline hover:text-zinc-100"
						>
							Remove image
						</button>
					</div>
				</div>
			{/if}
			<input
				bind:this={fileInput}
				type="file"
				accept="image/*"
				class="hidden"
				onchange={(e) => handleFile(e.currentTarget.files?.[0])}
			/>
			<Button
				variant="outline"
				class="w-full border-zinc-700 bg-zinc-800 text-zinc-100 hover:bg-zinc-700"
				onclick={() => fileInput?.click()}
			>
				<Upload />
				<span>{uploadPreview ? 'Choose a different image' : 'Choose image'}</span>
			</Button>
			{#if uploadError}
				<p class="text-xs text-red-400">{uploadError}</p>
			{/if}
			<p class="text-xs text-zinc-500">
				PNG or JPG under 800 KB works best. Square images look best.
			</p>
		</div>
	{/if}
</div>
