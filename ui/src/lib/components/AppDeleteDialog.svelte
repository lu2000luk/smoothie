<script lang="ts">
	import { CircleAlert, TriangleAlert } from 'lucide-svelte';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Card,
		CardDescription,
		CardHeader,
		CardPanel,
		CardTitle
	} from '$lib/components/ui/card/index.js';
	import { deleteApp, type App } from '$lib/apps';

	let {
		app,
		onclose,
		ondeleted
	}: {
		app: App | null;
		onclose: () => void;
		ondeleted: (id: string) => void;
	} = $props();

	let confirmName = $state('');
	let deleting = $state(false);
	let error = $state('');

	$effect(() => {
		if (app) {
			confirmName = '';
			error = '';
			deleting = false;
		}
	});

	const matches = $derived(app ? confirmName.trim() === app.name : false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!app || !matches || deleting) return;
		deleting = true;
		error = '';
		try {
			await deleteApp(app.id);
			ondeleted(app.id);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Could not delete app.';
			deleting = false;
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
					<CardTitle class="flex items-center gap-2">
						<TriangleAlert class="size-5 text-red-500" />
						Delete “{app.name}”?
					</CardTitle>
					<CardDescription>
						This permanently deletes the app and its services. Type the app name to confirm.
					</CardDescription>
				</CardHeader>
				<CardPanel>
					<form class="grid gap-4" onsubmit={submit}>
						<div class="grid gap-1">
							<label for="delete-confirm" class="text-sm font-medium">
								Type <span class="font-mono font-semibold">{app.name}</span> to confirm
							</label>
							<input
								id="delete-confirm"
								bind:value={confirmName}
								autocomplete="off"
								placeholder={app.name}
								class="h-9 rounded-lg border border-zinc-300 bg-white px-3 text-sm outline-none focus:border-red-500 focus:ring-2 focus:ring-red-500/30 dark:border-zinc-700 dark:bg-zinc-950"
							/>
						</div>
						{#if error}
							<Alert variant="error">
								<CircleAlert />
								<AlertDescription>{error}</AlertDescription>
							</Alert>
						{/if}
						<div class="flex justify-end gap-2">
							<Button variant="outline" onclick={onclose}>Cancel</Button>
							<Button variant="destructive" type="submit" loading={deleting} disabled={!matches}>
								Delete app
							</Button>
						</div>
					</form>
				</CardPanel>
			</Card>
		</div>
	</div>
{/if}
