<script lang="ts">
	import { CircleAlert, PackageOpen, Save, Trash2, TriangleAlert, X } from 'lucide-svelte';
	import {
		deleteService,
		updateService,
		type Service,
		type ServicePackage
	} from '$lib/api/services';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';

	let {
		appId,
		service,
		packages,
		onupdated,
		ondeleted
	}: {
		appId: string;
		service: Service;
		packages: ServicePackage[];
		onupdated: (service: Service) => void;
		ondeleted: (serviceId: string) => void;
	} = $props();

	let name = $state('');
	let port = $state('');
	let argv = $state('');
	let saving = $state(false);
	let clearing = $state(false);
	let deleting = $state(false);
	let error = $state('');
	let packageError = $state('');
	let clearConfirmOpen = $state(false);
	let deleteConfirm = $state('');

	const activePackage = $derived(
		packages.find((item) => item.id === service.active_package_id) ?? null
	);
	const deleteMatches = $derived(deleteConfirm.trim() === service.name);

	$effect(() => {
		service.id;
		service.updated_at;
		name = service.name;
		port = service.container_port?.toString() ?? '';
		argv = Array.isArray(service.argv) ? service.argv.join('\n') : (service.argv ?? '');
		deleteConfirm = '';
		clearConfirmOpen = false;
		error = '';
		packageError = '';
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!name.trim() || saving) return;
		const parsedPort = Number(port);
		if (!port.trim() || !Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535) {
			error = 'Port must be a whole number from 1 to 65535.';
			return;
		}
		saving = true;
		error = '';
		try {
			const updated = await updateService(appId, service.id, {
				name: name.trim(),
				container_port: parsedPort,
				argv: argv
					.split('\n')
					.map((argument) => argument.trim())
					.filter(Boolean)
			});
			onupdated(updated);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not save service settings.';
		} finally {
			saving = false;
		}
	}

	async function clearPackage() {
		if (!service.active_package_id || clearing) return;
		clearing = true;
		packageError = '';
		try {
			const updated = await updateService(appId, service.id, { active_package_id: null });
			clearConfirmOpen = false;
			onupdated(updated);
		} catch (caught) {
			packageError = caught instanceof Error ? caught.message : 'Could not clear active package.';
		} finally {
			clearing = false;
		}
	}

	async function remove(event: SubmitEvent) {
		event.preventDefault();
		if (!deleteMatches || deleting) return;
		deleting = true;
		error = '';
		try {
			await deleteService(appId, service.id);
			ondeleted(service.id);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not delete service.';
			deleting = false;
		}
	}
</script>

<div class="grid gap-7">
	<form class="grid gap-4" onsubmit={save}>
		<div>
			<h3 class="text-sm font-semibold text-zinc-100">Service settings</h3>
			<p class="mt-1 text-xs text-zinc-500">
				Configure how this service starts and receives traffic.
			</p>
		</div>

		<div class="grid gap-1.5">
			<label for="settings-service-name" class="text-xs font-medium text-zinc-300">Name</label>
			<input
				id="settings-service-name"
				bind:value={name}
				maxlength={64}
				autocomplete="off"
				class="h-9 rounded-lg border border-white/10 bg-zinc-950 px-3 text-sm text-zinc-100 outline-none focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20"
			/>
		</div>

		<div class="grid gap-1.5">
			<label for="settings-service-port" class="text-xs font-medium text-zinc-300"
				>Container port</label
			>
			<input
				id="settings-service-port"
				bind:value={port}
				type="number"
				min="1"
				max="65535"
				inputmode="numeric"
				required
				placeholder="8080"
				class="h-9 rounded-lg border border-white/10 bg-zinc-950 px-3 font-mono text-sm text-zinc-100 outline-none placeholder:text-zinc-700 focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20"
			/>
		</div>

		<div class="grid gap-1.5">
			<label for="settings-service-argv" class="text-xs font-medium text-zinc-300">Arguments</label>
			<textarea
				id="settings-service-argv"
				bind:value={argv}
				rows="4"
				spellcheck="false"
				placeholder={'--production\n--workers=2'}
				class="resize-y rounded-lg border border-white/10 bg-zinc-950 px-3 py-2 font-mono text-sm text-zinc-100 outline-none placeholder:text-zinc-700 focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20"
			></textarea>
			<p class="text-[11px] text-zinc-500">Enter one argument per line; each is passed to <code>./main</code>.</p>
		</div>

		{#if error}
			<Alert variant="error" class="text-zinc-200">
				<CircleAlert />
				<AlertDescription>{error}</AlertDescription>
			</Alert>
		{/if}

		<div>
			<Button type="submit" loading={saving} disabled={!name.trim()}>
				<Save /> Save settings
			</Button>
		</div>
	</form>

	<section class="grid gap-3 border-t border-white/8 pt-6" aria-labelledby="active-package-heading">
		<div>
			<h3 id="active-package-heading" class="text-sm font-semibold text-zinc-100">
				Active package
			</h3>
			<p class="mt-1 text-xs text-zinc-500">The package used for the next deployment.</p>
		</div>
		<div class="flex items-center gap-3 rounded-xl border border-white/8 bg-zinc-950/60 p-3">
			<div
				class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-white/5 text-zinc-400"
			>
				<PackageOpen class="size-4" aria-hidden="true" />
			</div>
			<div class="min-w-0 flex-1">
				{#if service.active_package_id}
					<p class="truncate text-xs font-medium text-zinc-200">
						{activePackage?.filename ?? service.active_package_id}
					</p>
					<p class="mt-1 truncate font-mono text-[10px] text-zinc-600">
						{service.active_package_id}
					</p>
				{:else}
					<p class="text-xs text-zinc-500">No package selected</p>
				{/if}
			</div>
			{#if service.active_package_id && !clearConfirmOpen}
				<Button
					variant="ghost"
					size="sm"
					class="text-zinc-300 hover:bg-white/8"
					onclick={() => (clearConfirmOpen = true)}
				>
					<X /> Clear
				</Button>
			{/if}
		</div>
		{#if service.active_package_id && clearConfirmOpen}
			<div class="grid gap-3 rounded-xl border border-amber-500/20 bg-amber-500/5 p-3">
				<div class="flex items-start gap-2 text-xs text-zinc-300">
					<TriangleAlert class="mt-0.5 size-3.5 shrink-0 text-amber-300" aria-hidden="true" />
					<p>Clear the active package? This also stops the running deployment.</p>
				</div>
				<div class="flex justify-end gap-2">
					<Button
						variant="ghost"
						size="sm"
						class="text-zinc-300 hover:bg-white/8"
						disabled={clearing}
						onclick={() => (clearConfirmOpen = false)}
					>
						Cancel
					</Button>
					<Button variant="destructive-outline" size="sm" loading={clearing} onclick={clearPackage}>
						Clear package
					</Button>
				</div>
			</div>
		{/if}
		{#if packageError}
			<Alert variant="error" class="text-zinc-200">
				<CircleAlert />
				<AlertDescription>{packageError}</AlertDescription>
			</Alert>
		{/if}
	</section>

	<section class="grid gap-4 border-t border-red-500/15 pt-6" aria-labelledby="danger-heading">
		<div class="flex items-start gap-2">
			<TriangleAlert class="mt-0.5 size-4 shrink-0 text-red-400" aria-hidden="true" />
			<div>
				<h3 id="danger-heading" class="text-sm font-semibold text-red-300">Delete service</h3>
				<p class="mt-1 text-xs text-zinc-500">
					This permanently removes the service, its deployments, and package history.
				</p>
			</div>
		</div>
		<form class="grid gap-3" onsubmit={remove}>
			<div class="grid gap-1.5">
				<label for="delete-service-confirm" class="text-xs font-medium text-zinc-300">
					Type <span class="font-mono text-zinc-100">{service.name}</span> to confirm
				</label>
				<input
					id="delete-service-confirm"
					bind:value={deleteConfirm}
					autocomplete="off"
					placeholder={service.name}
					class="h-9 rounded-lg border border-red-500/20 bg-zinc-950 px-3 text-sm text-zinc-100 outline-none placeholder:text-zinc-700 focus:border-red-400 focus:ring-2 focus:ring-red-400/20"
				/>
			</div>
			<div>
				<Button variant="destructive" type="submit" loading={deleting} disabled={!deleteMatches}>
					<Trash2 /> Delete service
				</Button>
			</div>
		</form>
	</section>
</div>
