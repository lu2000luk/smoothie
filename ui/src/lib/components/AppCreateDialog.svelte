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
	import { createApp, type App } from '$lib/apps';
	import type { IconSelection } from '$lib/appIcons';

	let {
		open,
		onclose,
		oncreated
	}: {
		open: boolean;
		onclose: () => void;
		oncreated: (app: App) => void;
	} = $props();

	let name = $state('');
	let icon: IconSelection = $state({ kind: null, value: null });
	let pickerOpen = $state(false);
	let saving = $state(false);
	let error = $state('');

	function reset() {
		name = '';
		icon = { kind: null, value: null };
		pickerOpen = false;
		saving = false;
		error = '';
	}

	function close() {
		reset();
		onclose();
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!name.trim() || saving) return;
		saving = true;
		error = '';
		try {
			const app = await createApp(name.trim(), icon.value, icon.kind);
			const created = app;
			reset();
			oncreated(created);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Could not create app.';
			saving = false;
		}
	}
</script>

{#if open}
	<div
		class="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4"
		role="presentation"
		onclick={close}
	>
		<div role="presentation" onclick={(e) => e.stopPropagation()}>
			<Card class="w-full max-w-sm">
				<CardHeader>
					<CardTitle>New app</CardTitle>
					<CardDescription>Give your app a name and an icon.</CardDescription>
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
								<label for="new-app-name" class="text-sm font-medium">Name</label>
								<input
									id="new-app-name"
									bind:value={name}
									placeholder="my-awesome-app"
									maxlength={64}
									autocomplete="off"
									class="h-9 rounded-lg border border-zinc-300 bg-white px-3 text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/30 dark:border-zinc-700 dark:bg-zinc-950"
								/>
							</div>
						</div>
						{#if pickerOpen}
							<AppIconPicker
								value={icon}
								onselect={(sel) => {
									icon = sel;
								}}
							/>
						{/if}
						{#if error}
							<Alert variant="error">
								<CircleAlert />
								<AlertDescription>{error}</AlertDescription>
							</Alert>
						{/if}
						<div class="flex justify-end gap-2">
							<Button variant="outline" onclick={close}>Cancel</Button>
							<Button type="submit" loading={saving} disabled={!name.trim()}>Create app</Button>
						</div>
					</form>
				</CardPanel>
			</Card>
		</div>
	</div>
{/if}
