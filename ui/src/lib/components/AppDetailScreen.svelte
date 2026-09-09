<script lang="ts">
	import { ArrowLeft, LogOut } from 'lucide-svelte';
	import type { AuthUser } from '$lib/auth';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import AppIconPicker from './AppIconPicker.svelte';
	import ServiceWorkspace from './ServiceWorkspace.svelte';
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
		} catch (caught) {
			loadError = caught instanceof Error ? caught.message : 'Could not load app.';
			status = 'error';
		}
	}

	$effect(() => {
		if (appId) load();
	});

	function handleDialogKey(event: KeyboardEvent) {
		if (iconPickerOpen && event.key === 'Escape') iconPickerOpen = false;
	}

	async function handleIconSelect(selection: IconSelection) {
		if (!app || savingIcon) return;
		if (selection.kind === (app.icon_kind ?? null) && selection.value === (app.icon ?? null)) {
			iconPickerOpen = false;
			return;
		}
		savingIcon = true;
		iconError = '';
		try {
			app = await renameApp(app.id, app.name, selection.value, selection.kind);
			iconPickerOpen = false;
		} catch (caught) {
			iconError = caught instanceof Error ? caught.message : 'Could not update icon.';
		} finally {
			savingIcon = false;
		}
	}
</script>

<svelte:window onkeydown={handleDialogKey} />

<div class="flex h-dvh min-h-0 flex-col overflow-hidden bg-[#0d0d0f] text-zinc-100">
	<header class="z-50 shrink-0 border-b border-white/8 bg-zinc-950/95 backdrop-blur">
		<div class="flex h-14 items-center gap-3 px-3 sm:px-4">
			<Button
				variant="ghost"
				size="icon-sm"
				href="/"
				class="text-zinc-300 hover:bg-white/8"
				aria-label="Back to apps"
			>
				<ArrowLeft />
			</Button>
			<div class="h-5 w-px bg-white/10"></div>
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
					class="shrink-0 rounded-lg outline-none transition-opacity hover:opacity-80 focus-visible:ring-2 focus-visible:ring-violet-400 disabled:opacity-60"
				>
					<AppIconDisplay kind={app.icon_kind} value={app.icon} size="sm" />
				</button>
				<div class="min-w-0 flex-1">
					<p class="truncate text-sm font-semibold text-zinc-100">{app.name}</p>
					<p class="hidden truncate font-mono text-[10px] text-zinc-600 sm:block">{app.id}</p>
				</div>
			{:else}
				<div class="min-w-0 flex-1">
					<p class="truncate text-sm font-semibold">App</p>
					<p class="hidden truncate font-mono text-[10px] text-zinc-600 sm:block">{appId}</p>
				</div>
			{/if}
			{#if iconError}
				<p class="hidden max-w-xs truncate text-xs text-red-300 md:block" role="alert">
					{iconError}
				</p>
			{/if}
			<Avatar class="size-8 border border-white/8">
				{#if user.avatar_url}
					<AvatarImage src={user.avatar_url} alt={user.login} />
				{:else}
					<AvatarFallback>{initials}</AvatarFallback>
				{/if}
			</Avatar>
			<Button variant="ghost" size="sm" onclick={onlogout} class="text-zinc-300 hover:bg-white/8">
				<LogOut />
				<span class="hidden sm:inline">Log out</span>
			</Button>
		</div>
	</header>

	<main class="flex min-h-0 flex-1">
		{#if status === 'loading'}
			<div class="grid flex-1 place-items-center">
				<Spinner class="size-7 text-violet-300" />
			</div>
		{:else if status === 'error'}
			<div class="grid flex-1 place-items-center p-6">
				<div
					class="grid max-w-sm justify-items-center gap-3 rounded-xl border border-white/10 bg-zinc-900 p-6 text-center"
				>
					<p class="text-sm text-red-300">{loadError}</p>
					<Button
						variant="outline"
						class="border-white/10 bg-white/5 text-zinc-100 hover:bg-white/10"
						onclick={load}
					>
						Try again
					</Button>
				</div>
			</div>
		{:else}
			<ServiceWorkspace {appId} />
		{/if}
	</main>
</div>

{#if iconPickerOpen && app}
	<div
		class="fixed inset-0 z-90 flex overflow-y-auto bg-black/70 p-4"
		role="presentation"
		onclick={() => {
			if (!savingIcon) iconPickerOpen = false;
		}}
	>
		<div class="m-auto" role="presentation" onclick={(event) => event.stopPropagation()}>
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
