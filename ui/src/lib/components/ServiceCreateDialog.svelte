<script lang="ts">
	import { CircleAlert, PackageOpen } from 'lucide-svelte';
	import { createService, type Service } from '$lib/api/services';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Card,
		CardDescription,
		CardHeader,
		CardPanel,
		CardTitle
	} from '$lib/components/ui/card/index.js';

	let {
		open,
		appId,
		position,
		onclose,
		oncreated
	}: {
		open: boolean;
		appId: string;
		position: { x: number; y: number };
		onclose: () => void;
		oncreated: (service: Service) => void;
	} = $props();

	let name = $state('');
	let saving = $state(false);
	let error = $state('');

	function close() {
		if (saving) return;
		name = '';
		error = '';
		onclose();
	}

	function handleKey(event: KeyboardEvent) {
		if (open && event.key === 'Escape') close();
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (!name.trim() || saving) return;
		saving = true;
		error = '';
		try {
			const service = await createService(appId, {
				name: name.trim(),
				position_x: Math.round(position.x),
				position_y: Math.round(position.y)
			});
			name = '';
			oncreated(service);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not create service.';
		} finally {
			saving = false;
		}
	}
</script>

<svelte:window onkeydown={handleKey} />

{#if open}
	<div
		class="fixed inset-0 z-80 flex overflow-y-auto bg-black/70 p-4"
		role="presentation"
		onclick={close}
	>
		<div class="m-auto" role="presentation" onclick={(event) => event.stopPropagation()}>
			<Card
				class="w-105 max-w-[calc(100vw-2rem)] border-white/10 bg-zinc-900 text-zinc-100 shadow-2xl"
			>
				<CardHeader>
					<CardTitle>New service</CardTitle>
					<CardDescription class="text-zinc-400">
						Create an empty service, then upload a package when you are ready.
					</CardDescription>
				</CardHeader>
				<CardPanel>
					<form class="grid gap-4" onsubmit={submit}>
						<div class="grid gap-1.5">
							<label for="service-name" class="text-sm font-medium">Service name</label>
							<input
								id="service-name"
								bind:value={name}
								placeholder="web"
								maxlength={64}
								autocomplete="off"
								class="h-9 rounded-lg border border-white/10 bg-zinc-950 px-3 text-sm text-zinc-100 outline-none placeholder:text-zinc-600 focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20"
							/>
						</div>
						<div
							class="flex items-start gap-2 rounded-lg border border-white/8 bg-white/3 p-3 text-xs text-zinc-400"
						>
							<PackageOpen class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
							No image or package is required to create this service.
						</div>
						{#if error}
							<Alert variant="error">
								<CircleAlert />
								<AlertDescription>{error}</AlertDescription>
							</Alert>
						{/if}
						<div class="flex justify-end gap-2">
							<Button variant="outline" onclick={close}>Cancel</Button>
							<Button type="submit" loading={saving} disabled={!name.trim()}>Create service</Button>
						</div>
					</form>
				</CardPanel>
			</Card>
		</div>
	</div>
{/if}
