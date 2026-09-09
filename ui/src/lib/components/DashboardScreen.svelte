<script lang="ts">
	import { LogOut, Plus, Search } from 'lucide-svelte';
	import type { AuthUser } from '$lib/auth';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import AppCard from './AppCard.svelte';
	import AppCreateDialog from './AppCreateDialog.svelte';
	import { listApps, type App } from '$lib/apps';

	let { user, onlogout }: { user: AuthUser; onlogout: () => void } = $props();

	let apps = $state<App[]>([]);
	let status = $state<'loading' | 'ready' | 'error'>('loading');
	let loadError = $state('');
	let query = $state('');
	let createOpen = $state(false);

	const initials = $derived(user.login.slice(0, 2).toUpperCase());

	async function load() {
		status = 'loading';
		loadError = '';
		try {
			apps = await listApps();
			status = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Could not load apps.';
			status = 'error';
		}
	}

	$effect(() => {
		load();
	});

	const filtered = $derived.by(() => {
		const q = query.trim().toLowerCase();
		if (!q) return apps;
		return apps.filter((a) => a.name.toLowerCase().includes(q) || a.id.toLowerCase().includes(q));
	});
</script>

<div class="bg-background min-h-screen">
	<header class="border-b">
		<div class="mx-auto flex max-w-4xl items-center gap-3 px-4 py-3">
			<Avatar class="size-9">
				{#if user.avatar_url}
					<AvatarImage src={user.avatar_url} alt={user.login} />
				{:else}
					<AvatarFallback>{initials}</AvatarFallback>
				{/if}
			</Avatar>
			<div class="min-w-0 flex-1">
				<p class="truncate text-sm font-semibold">{user.name ?? user.login}</p>
				<p class="truncate text-xs text-zinc-500">@{user.login}</p>
			</div>
			<Button variant="outline" size="sm" onclick={onlogout}>
				<LogOut />
				<span>Log out</span>
			</Button>
		</div>
	</header>

	<main class="mx-auto grid max-w-4xl gap-4 px-4 py-6">
		<div class="flex flex-col gap-3 sm:flex-row sm:items-center">
			<div class="flex-1">
				<h1 class="text-xl font-semibold tracking-tight">Apps</h1>
				<p class="text-sm text-zinc-500">{apps.length} app{apps.length === 1 ? '' : 's'}</p>
			</div>
			<label class="flex h-9 flex-1 items-center gap-2 rounded-lg border px-3 sm:max-w-xs">
				<Search class="size-4 text-zinc-400" />
				<input
					bind:value={query}
					placeholder="Filter apps..."
					class="w-full bg-transparent text-sm outline-none placeholder:text-zinc-400"
				/>
			</label>
			<Button onclick={() => (createOpen = true)}>
				<Plus />
				<span>New app</span>
			</Button>
		</div>

		{#if status === 'loading'}
			<div class="grid place-items-center py-16">
				<Spinner class="size-8" />
			</div>
		{:else if status === 'error'}
			<div class="grid gap-3 rounded-2xl border p-8 text-center">
				<p class="text-sm text-red-600">{loadError}</p>
				<div class="flex justify-center">
					<Button variant="outline" onclick={load}>Try again</Button>
				</div>
			</div>
		{:else if filtered.length === 0 && !query}
			<div class="grid gap-2 rounded-2xl border border-dashed p-10 text-center">
				<p class="font-medium">No apps yet</p>
				<p class="text-sm text-zinc-500">Create your first app to get started.</p>
				<div class="mt-2 flex justify-center">
					<Button onclick={() => (createOpen = true)}>
						<Plus />
						<span>Create app</span>
					</Button>
				</div>
			</div>
		{:else if filtered.length === 0}
			<p class="py-10 text-center text-sm text-zinc-500">No apps match “{query}”.</p>
		{:else}
			<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
				{#each filtered as app (app.id)}
					<AppCard {app} />
				{/each}
			</div>
		{/if}
	</main>

	<AppCreateDialog
		open={createOpen}
		onclose={() => (createOpen = false)}
		oncreated={(app) => {
			apps = [app, ...apps];
			createOpen = false;
		}}
	/>
</div>
