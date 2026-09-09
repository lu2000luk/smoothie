<script lang="ts">
	import {
		Apple,
		Check,
		Clock,
		Flag,
		Leaf,
		Lightbulb,
		Plane,
		Search,
		Shuffle,
		Smile,
		Trophy,
		X
	} from 'lucide-svelte';
	import {
		EMOJI_CATEGORIES,
		getRecentIcons,
		pushRecentIcon,
		randomEmoji,
		searchEmojis,
		type IconSelection
	} from '$lib/appIcons';
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

	let query = $state('');
	let recent = $state<IconSelection[]>(getRecentIcons().filter((item) => item.kind === 'emoji'));
	let activeSection = $state('recent');
	let scrollEl: HTMLDivElement | null = $state(null);
	let filterEl: HTMLInputElement | null = $state(null);

	$effect(() => {
		filterEl?.focus();
	});

	const nav = [
		{ id: 'recent', label: 'Recent', icon: Clock },
		{ id: 'people', label: 'People', icon: Smile },
		{ id: 'nature', label: 'Nature', icon: Leaf },
		{ id: 'food', label: 'Food', icon: Apple },
		{ id: 'activity', label: 'Activity', icon: Trophy },
		{ id: 'travel', label: 'Travel', icon: Plane },
		{ id: 'objects', label: 'Objects', icon: Lightbulb },
		{ id: 'symbols', label: 'Symbols', icon: Check },
		{ id: 'flags', label: 'Flags', icon: Flag }
	];

	function pick(sel: IconSelection) {
		recent = pushRecentIcon(sel).filter((item) => item.kind === 'emoji');
		onselect(sel);
	}

	function handleRemove() {
		onselect({ kind: null, value: null });
		onclose?.();
	}

	function handleRandom() {
		pick({ kind: 'emoji', value: randomEmoji() });
	}

	function scrollToSection(id: string) {
		query = '';
		activeSection = id;
		requestAnimationFrame(() => {
			const section = scrollEl?.querySelector<HTMLElement>(`[data-section="${id}"]`);
			if (scrollEl && section) scrollEl.scrollTo({ top: section.offsetTop, behavior: 'smooth' });
		});
	}

	function trackActiveSection() {
		if (!scrollEl || query) return;
		const sections = [...scrollEl.querySelectorAll<HTMLElement>('[data-section]')];
		const current = sections.findLast((section) => section.offsetTop <= scrollEl!.scrollTop + 12);
		activeSection = current?.dataset.section ?? 'recent';
	}

	const results = $derived(searchEmojis(query));
	const isSelected = $derived((emoji: string) => value.kind === 'emoji' && value.value === emoji);
</script>

<div
	class="w-[410px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-zinc-700 bg-zinc-900 text-zinc-100 shadow-2xl"
	role="dialog"
	aria-label="Choose app icon"
>
	<div class="flex items-center justify-end px-4 pt-3">
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
				bind:this={filterEl}
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
	</div>

	<div
		bind:this={scrollEl}
		onscroll={trackActiveSection}
		class="h-64 overflow-y-auto scroll-smooth px-3 pb-2"
	>
		{#if results}
			{#if results.length === 0}
				<p class="py-8 text-center text-sm text-zinc-500">
					No emoji match “{query.trim()}”.
				</p>
			{:else}
				<p class="py-1.5 text-xs font-medium text-zinc-400">Search results</p>
				<div class="grid grid-cols-12 gap-0.5">
					{#each results as emoji, i (`search-${emoji}-${i}`)}
						<button
							type="button"
							onclick={() => pick({ kind: 'emoji', value: emoji })}
							aria-label={`Use ${emoji} as icon`}
							class={cn(
								'flex h-8 w-full items-center justify-center rounded-md text-xl transition-colors hover:bg-zinc-700',
								isSelected(emoji) && 'bg-blue-500/25 outline-1 outline-blue-500'
							)}
						>
							{emoji}
						</button>
					{/each}
				</div>
			{/if}
		{:else}
			<p data-section="recent" class="scroll-mt-1 py-1.5 text-xs font-medium text-zinc-400">
				Recent
			</p>
			<div class="flex flex-wrap gap-0.5">
				{#each recent as item, i (`${item.kind}-${item.value}-${i}`)}
					{#if item.value}
						<button
							type="button"
							onclick={() => item.value && pick({ kind: 'emoji', value: item.value })}
							aria-label={`Use ${item.value} as icon`}
							class={cn(
								'flex size-9 items-center justify-center rounded-md bg-zinc-800 text-xl transition-colors hover:bg-zinc-700',
								item.kind === 'emoji' &&
									isSelected(item.value) &&
									'bg-blue-500/25 outline-1 outline-blue-500'
							)}
						>
							{item.value}
						</button>
					{/if}
				{/each}
			</div>
			{#each EMOJI_CATEGORIES as category (category.id)}
				<p data-section={category.id} class="scroll-mt-1 py-1.5 text-xs font-medium text-zinc-400">
					{category.label}
				</p>
				<div class="grid grid-cols-12 gap-0.5">
					{#each category.emojis as emoji, i (`${category.id}-${emoji}-${i}`)}
						<button
							type="button"
							onclick={() => pick({ kind: 'emoji', value: emoji })}
							aria-label={`Use ${emoji} as icon`}
							class={cn(
								'flex h-8 w-full items-center justify-center rounded-md text-xl transition-colors hover:bg-zinc-700',
								isSelected(emoji) && 'bg-blue-500/25 outline-1 outline-blue-500'
							)}
						>
							{emoji}
						</button>
					{/each}
				</div>
			{/each}
		{/if}
	</div>

	<div class="flex items-center justify-between border-t border-zinc-800 bg-zinc-900 px-2 py-2">
		{#each nav as item (item.id)}
			<button
				type="button"
				onclick={() => scrollToSection(item.id)}
				title={item.label}
				aria-label={item.label}
				aria-current={activeSection === item.id ? 'true' : undefined}
				class={cn(
					'flex size-8 items-center justify-center rounded-md text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200',
					activeSection === item.id && 'bg-zinc-800 text-zinc-200'
				)}
			>
				<item.icon class="size-4" />
			</button>
		{/each}
	</div>
</div>
