<script lang="ts">
	import { ArrowLeft, LogOut } from 'lucide-svelte';
	import type { AuthUser } from '$lib/auth';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import AppIconPicker from './AppIconPicker.svelte';
	import { getApp, renameApp, type App } from '$lib/apps';
	import type { IconSelection } from '$lib/appIcons';

	let {
		user,
		appId,
		onlogout
	}: {
		user: AuthUser;
		appId: string;
		onlogout: () => void;
	} = $props();

	let app = $state<App | null>(null);
	let status = $state<'loading' | 'ready' | 'error'>('loading');
	let loadError = $state('');
	let iconPickerOpen = $state(false);
	let savingIcon = $state(false);
	let iconError = $state('');

	const initials = $derived(user.login.slice(0, 2).toUpperCase());

	async function load() {
		status = 'loading';
		loadError = '';
		try {
			app = await getApp(appId);
			status = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Could not load app.';
			status = 'error';
		}
	}

	$effect(() => {
		if (appId) load();
	});

	function handleDialogKey(e: KeyboardEvent) {
		if (!iconPickerOpen || e.key !== 'Escape') return;
		iconPickerOpen = false;
	}

	async function handleIconSelect(sel: IconSelection) {
		if (!app || savingIcon) return;
		if (sel.kind === (app.icon_kind ?? null) && sel.value === (app.icon ?? null)) {
			iconPickerOpen = false;
			return;
		}
		savingIcon = true;
		iconError = '';
		try {
			app = await renameApp(app.id, app.name, sel.value, sel.kind);
			iconPickerOpen = false;
		} catch (err) {
			iconError = err instanceof Error ? err.message : 'Could not update icon.';
		} finally {
			savingIcon = false;
		}
	}
</script>

<svelte:window onkeydown={handleDialogKey} />

<div class="bg-background min-h-screen">
	<header class="border-b">
		<div class="mx-auto flex max-w-4xl items-center gap-3 px-4 py-3">
			<Button variant="ghost" size="icon-sm" href="/">
				<ArrowLeft />
			</Button>
			{#if app}
				<button
					type="button"
					onclick={() => {
						iconError = '';
						iconPickerOpen = true;
					}}
					disabled={savingIcon}
					title="Edit icon"
					aria-label="Edit app icon"
					aria-haspopup="dialog"
					class="shrink-0 rounded-xl outline-none transition-opacity focus-visible:ring-2 focus-visible:ring-blue-500 hover:opacity-80 disabled:opacity-60"
				>
					<AppIconDisplay kind={app.icon_kind} value={app.icon} size="sm" />
				</button>
				<div class="min-w-0 flex-1">
					<p class="truncate text-sm font-semibold">{app.name}</p>
					<p class="truncate font-mono text-xs text-zinc-500">{app.id}</p>
				</div>
			{:else}
				<div class="min-w-0 flex-1">
					<p class="truncate text-sm font-semibold">App</p>
					<p class="truncate font-mono text-xs text-zinc-500">{appId}</p>
				</div>
			{/if}
			<Avatar class="size-9">
				{#if user.avatar_url}
					<AvatarImage src={user.avatar_url} alt={user.login} />
				{:else}
					<AvatarFallback>{initials}</AvatarFallback>
				{/if}
			</Avatar>
			<Button variant="outline" size="sm" onclick={onlogout}>
				<LogOut />
				<span>Log out</span>
			</Button>
		</div>
	</header>

	<main class="mx-auto grid max-w-4xl gap-4 px-4 py-6">
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
		{:else}
			{#if iconError}
				<p class="text-sm text-red-600 dark:text-red-400">{iconError}</p>
			{/if}
			<!-- Empty app page. Per-app content will go here. -->
		{/if}
	</main>
</div>

{#if iconPickerOpen && app}
	<div
		class="fixed inset-0 z-[70] flex overflow-y-auto bg-black/60 p-4"
		role="presentation"
		onclick={() => {
			if (!savingIcon) iconPickerOpen = false;
		}}
	>
		<div class="m-auto" role="presentation" onclick={(e) => e.stopPropagation()}>
			<AppIconPicker
				value={{ kind: app.icon_kind ?? null, value: app.icon ?? null }}
				onselect={handleIconSelect}
				onclose={() => {
					if (!savingIcon) iconPickerOpen = false;
				}}
			/>
		</div>
	</div>
{/if}
