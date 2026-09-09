<script lang="ts">
	import { CircleAlert } from 'lucide-svelte';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Card,
		CardDescription,
		CardHeader,
		CardPanel,
		CardTitle
	} from '$lib/components/ui/card/index.js';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import AppIconPicker from './AppIconPicker.svelte';
	import { renameApp, type App } from '$lib/apps';
	import type { IconSelection } from '$lib/appIcons';

	let {
		app,
		onclose,
		onrenamed
	}: {
		app: App | null;
		onclose: () => void;
		onrenamed: (app: App) => void;
	} = $props();

	let name = $state('');
	let icon: IconSelection = $state({ kind: null, value: null });
	let pickerOpen = $state(false);
	let saving = $state(false);
	let error = $state('');

	$effect(() => {
		if (app) {
			name = app.name;
			icon = { kind: app.icon_kind ?? null, value: app.icon ?? null };
			pickerOpen = false;
			error = '';
			saving = false;
		}
	});

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!app || !name.trim() || saving) return;
		saving = true;
		error = '';
		try {
			const updated = await renameApp(app.id, name.trim(), icon.value, icon.kind);
			onrenamed(updated);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Could not rename app.';
			saving = false;
		}
	}
</script>

{#if app}
	<div
		class="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4"
		role="presentation"
		onclick={onclose}
	>
		<div role="presentation" onclick={(e) => e.stopPropagation()}>
			<Card class="w-full max-w-sm">
				<CardHeader>
					<CardTitle>Rename app</CardTitle>
					<CardDescription
						>ID cannot be changed. Only the name and icon are editable.</CardDescription
					>
				</CardHeader>
				<CardPanel>
					<form class="grid gap-4" onsubmit={submit}>
						<div class="flex items-center gap-3">
							<button
								type="button"
								onclick={() => (pickerOpen = !pickerOpen)}
								class="shrink-0 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
								aria-label="Choose app icon"
							>
								<AppIconDisplay kind={icon.kind} value={icon.value} size="lg" class="border" />
							</button>
							<div class="grid flex-1 gap-1">
								<label for="rename-app-name" class="text-sm font-medium">Name</label>
								<input
									id="rename-app-name"
									bind:value={name}
									maxlength={64}
									autocomplete="off"
									class="h-9 rounded-lg border border-zinc-300 bg-white px-3 text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/30 dark:border-zinc-700 dark:bg-zinc-950"
								/>
							</div>
						</div>
						{#if pickerOpen}
							<AppIconPicker value={icon} onselect={(sel) => (icon = sel)} />
						{/if}
						<p class="truncate font-mono text-xs text-zinc-500">id: {app.id}</p>
						{#if error}
							<Alert variant="error">
								<CircleAlert />
								<AlertDescription>{error}</AlertDescription>
							</Alert>
						{/if}
						<div class="flex justify-end gap-2">
							<Button variant="outline" onclick={onclose}>Cancel</Button>
							<Button type="submit" loading={saving} disabled={!name.trim()}>Save changes</Button>
						</div>
					</form>
				</CardPanel>
			</Card>
		</div>
	</div>
{/if}
