<script lang="ts">
	import {
		Archive,
		Check,
		CircleAlert,
		Copy,
		Database,
		ExternalLink,
		Package as PackageIcon,
		Play,
		RefreshCcw,
		Rocket,
		Square,
		Trash2,
		Upload
	} from 'lucide-svelte';
	import {
		activateServicePackage,
		currentDeployment,
		deleteServicePackage,
		retryServiceDeployment,
		startServiceDeployment,
		stopServiceDeployment,
		uploadServicePackage,
		type Service,
		type ServicePackage,
		type ServiceState
	} from '$lib/api/services';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import ServiceStatusBadge from './ServiceStatusBadge.svelte';

	let {
		appId,
		service,
		status,
		packages,
		packagesLoading,
		packagesError,
		onreloadpackages,
		onrefresh,
		onserviceupdated,
		onbusychange
	}: {
		appId: string;
		service: Service;
		status: ServiceState;
		packages: ServicePackage[];
		packagesLoading: boolean;
		packagesError: string;
		onreloadpackages: () => Promise<void>;
		onrefresh: () => Promise<void>;
		onserviceupdated: (service: Service) => void;
		onbusychange: (status: ServiceState | null) => void;
	} = $props();

	const STORAGE_LIMIT = 200 * 1024 * 1024;
	let fileInput = $state<HTMLInputElement | null>(null);
	let uploading = $state(false);
	let deploymentBusy = $state(false);
	let busyPackageId = $state<string | null>(null);
	let error = $state('');
	let copied = $state(false);

	const deployment = $derived(currentDeployment(service));
	const deploymentId = $derived(deployment?.id ?? service.active_deployment_id);
	const endpoint = $derived.by(() => {
		const direct = deployment?.endpoint ?? deployment?.url ?? service.endpoint ?? service.url;
		if (direct) return direct;
		if (!deployment?.host) return null;
		const host = deployment.host.includes('://') ? deployment.host : `http://${deployment.host}`;
		return deployment.host_port ? `${host.replace(/\/$/, '')}:${deployment.host_port}` : host;
	});
	const usedStorage = $derived(
		packages
			.filter((item) => !['pruned', 'deleted'].includes(item.status.toLowerCase()))
			.reduce((total, item) => total + item.size, 0)
	);
	const storagePercent = $derived(Math.min(100, (usedStorage / STORAGE_LIMIT) * 100));
	const sortedPackages = $derived(
		[...packages].sort(
			(a, b) => new Date(b.uploaded_at).getTime() - new Date(a.uploaded_at).getTime()
		)
	);

	function formatSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
		return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
	}

	function formatDate(value?: string | null): string {
		if (!value) return 'Unknown time';
		const date = new Date(value);
		return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
	}

	function packageUnavailable(item: ServicePackage): boolean {
		return Boolean(item.pruned_at) || ['pruned', 'deleted'].includes(item.status.toLowerCase());
	}

	async function upload(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		if (!file || uploading) return;
		if (!file.name.toLowerCase().endsWith('.tar')) {
			error = 'Choose a .tar package.';
			input.value = '';
			return;
		}
		uploading = true;
		error = '';
		onbusychange('uploading');
		try {
			const result = await uploadServicePackage(appId, service.id, file);
			onserviceupdated(result.service);
			await Promise.all([onreloadpackages(), onrefresh()]);
			if (result.error) error = result.error;
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not upload package.';
		} finally {
			uploading = false;
			onbusychange(null);
			input.value = '';
		}
	}

	async function activate(item: ServicePackage) {
		if (busyPackageId || packageUnavailable(item)) return;
		busyPackageId = item.id;
		error = '';
		try {
			const result = await activateServicePackage(appId, service.id, item.id);
			onserviceupdated(result.service);
			await Promise.all([onreloadpackages(), onrefresh()]);
			if (result.error) error = result.error;
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not activate package.';
		} finally {
			busyPackageId = null;
		}
	}

	async function removePackage(item: ServicePackage) {
		if (busyPackageId || item.id === service.active_package_id) return;
		busyPackageId = item.id;
		error = '';
		try {
			await deleteServicePackage(appId, service.id, item.id);
			await Promise.all([onreloadpackages(), onrefresh()]);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not delete package.';
		} finally {
			busyPackageId = null;
		}
	}

	async function start(retry = false) {
		if (deploymentBusy || !service.active_package_id) return;
		deploymentBusy = true;
		error = '';
		onbusychange('deploying');
		try {
			const activeDeploymentId = service.active_deployment?.id ?? service.active_deployment_id;
			const result =
				retry && activeDeploymentId
					? await retryServiceDeployment(appId, service.id, activeDeploymentId)
					: await startServiceDeployment(appId, service.id);
			onserviceupdated(result.service);
			await onrefresh();
			if (result.error) error = result.error;
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not start deployment.';
		} finally {
			deploymentBusy = false;
			onbusychange(null);
		}
	}

	async function stop() {
		if (deploymentBusy) return;
		deploymentBusy = true;
		error = '';
		try {
			const result = await stopServiceDeployment(appId, service.id);
			onserviceupdated(result.service);
			await onrefresh();
		} catch (caught) {
			error = caught instanceof Error ? caught.message : 'Could not stop deployment.';
		} finally {
			deploymentBusy = false;
		}
	}

	async function copyEndpoint() {
		if (!endpoint) return;
		try {
			await navigator.clipboard.writeText(endpoint);
			copied = true;
			setTimeout(() => (copied = false), 1600);
		} catch {
			error = 'Could not copy the endpoint.';
		}
	}
</script>

<div class="grid gap-6">
	<section aria-labelledby="current-deployment-heading" class="grid gap-3">
		<div class="flex items-center justify-between gap-3">
			<h3 id="current-deployment-heading" class="text-sm font-semibold text-zinc-100">
				Current deployment
			</h3>
			<ServiceStatusBadge {status} />
		</div>
		<div class="rounded-xl border border-white/8 bg-zinc-950/70 p-4">
			{#if deploymentId}
				<div class="grid gap-3">
					<div class="flex items-start gap-3">
						<div
							class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-violet-500/12 text-violet-300"
						>
							<Rocket class="size-4" aria-hidden="true" />
						</div>
						<div class="min-w-0 flex-1">
							<p class="truncate font-mono text-xs text-zinc-300">{deploymentId}</p>
							<p class="mt-1 text-xs text-zinc-500">
								{#if deployment?.started_at || deployment?.created_at}
									Started {formatDate(deployment.started_at ?? deployment.created_at)}
								{:else}
									Active deployment
								{/if}
							</p>
						</div>
					</div>
					{#if endpoint}
						<div
							class="flex min-w-0 items-center gap-2 rounded-lg border border-white/6 bg-white/3 p-2"
						>
							<code class="min-w-0 flex-1 truncate text-xs text-zinc-300">{endpoint}</code>
							<Button
								variant="ghost"
								size="icon-xs"
								class="text-zinc-300 hover:bg-white/8"
								onclick={copyEndpoint}
								aria-label="Copy direct endpoint"
							>
								{#if copied}<Check />{:else}<Copy />{/if}
							</Button>
							<Button
								variant="ghost"
								size="icon-xs"
								class="text-zinc-300 hover:bg-white/8"
								href={endpoint}
								target="_blank"
								rel="noreferrer"
								aria-label="Open direct endpoint"
							>
								<ExternalLink />
							</Button>
						</div>
					{/if}
				</div>
			{:else}
				<div class="flex items-center gap-3 text-sm text-zinc-500">
					<Rocket class="size-4" aria-hidden="true" />
					No deployment has been started.
				</div>
			{/if}
		</div>
		<div class="flex flex-wrap gap-2">
			{#if status === 'running' || status === 'deploying'}
				<Button
					variant="outline"
					class="border-white/10 bg-white/5 text-zinc-100 hover:bg-white/10"
					loading={deploymentBusy}
					onclick={stop}
				>
					<Square /> Stop
				</Button>
			{:else if status === 'failed'}
				<Button
					loading={deploymentBusy}
					disabled={!service.active_package_id}
					onclick={() => start(true)}
				>
					<RefreshCcw /> Retry
				</Button>
			{:else}
				<Button
					loading={deploymentBusy}
					disabled={!service.active_package_id}
					onclick={() => start(false)}
				>
					<Play /> Start
				</Button>
			{/if}
			{#if !service.active_package_id}
				<p class="self-center text-xs text-zinc-500">Activate a package before starting.</p>
			{/if}
		</div>
	</section>

	<section aria-labelledby="packages-heading" class="grid gap-3">
		<div class="flex items-center justify-between gap-3">
			<div>
				<h3 id="packages-heading" class="text-sm font-semibold text-zinc-100">Packages</h3>
				<p class="mt-0.5 text-xs text-zinc-500">Upload a .tar archive to deploy this service.</p>
			</div>
			<input
				bind:this={fileInput}
				type="file"
				accept=".tar,application/x-tar"
				class="sr-only"
				onchange={upload}
				aria-label="Upload tar package"
			/>
			<Button
				variant="outline"
				size="sm"
				class="border-white/10 bg-white/5 text-zinc-100 hover:bg-white/10"
				loading={uploading}
				onclick={() => fileInput?.click()}
			>
				<Upload /> Upload
			</Button>
		</div>

		<div class="rounded-xl border border-white/8 bg-white/2 p-3">
			<div class="mb-2 flex items-center justify-between gap-3 text-xs">
				<span class="flex items-center gap-1.5 text-zinc-400">
					<Database class="size-3.5" aria-hidden="true" /> Storage usage
				</span>
				<span class="font-mono text-zinc-300">{formatSize(usedStorage)} / 200 MiB</span>
			</div>
			<div
				class="h-1.5 overflow-hidden rounded-full bg-zinc-800"
				role="progressbar"
				aria-label="Package storage usage"
				aria-valuemin="0"
				aria-valuemax="200"
				aria-valuenow={Math.round(usedStorage / (1024 * 1024))}
			>
				<div class="h-full rounded-full bg-violet-400" style={`width: ${storagePercent}%`}></div>
			</div>
		</div>

		{#if packagesError || error}
			<Alert variant="error" class="text-zinc-200">
				<CircleAlert />
				<AlertDescription>{error || packagesError}</AlertDescription>
			</Alert>
		{/if}

		{#if packagesLoading}
			<p class="py-6 text-center text-sm text-zinc-500">Loading package history…</p>
		{:else if sortedPackages.length === 0}
			<div
				class="grid place-items-center gap-2 rounded-xl border border-dashed border-white/10 py-8 text-center"
			>
				<PackageIcon class="size-5 text-zinc-600" aria-hidden="true" />
				<p class="text-sm text-zinc-400">No packages uploaded</p>
			</div>
		{:else}
			<ul class="grid gap-2" aria-label="Package history">
				{#each sortedPackages as item (item.id)}
					{@const unavailable = packageUnavailable(item)}
					{@const active = item.id === service.active_package_id}
					<li class="rounded-xl border border-white/8 bg-zinc-950/50 p-3">
						<div class="flex items-start gap-3">
							<div class="mt-0.5 text-zinc-500">
								{#if unavailable}<Archive class="size-4" />{:else}<PackageIcon
										class="size-4"
									/>{/if}
							</div>
							<div class="min-w-0 flex-1">
								<div class="flex flex-wrap items-center gap-2">
									<p class="max-w-full truncate text-xs font-medium text-zinc-200">
										{item.filename}
									</p>
									{#if active}
										<span
											class="rounded-full bg-emerald-500/12 px-2 py-0.5 text-[10px] font-medium text-emerald-300"
											>Active</span
										>
									{:else if unavailable}
										<span
											class="rounded-full bg-zinc-800 px-2 py-0.5 text-[10px] font-medium text-zinc-400"
										>
											{item.status.toLowerCase() === 'deleted' ? 'Deleted' : 'Pruned'}
										</span>
									{:else}
										<span
											class="rounded-full bg-violet-500/12 px-2 py-0.5 text-[10px] font-medium text-violet-300"
											>Available</span
										>
									{/if}
								</div>
								<p class="mt-1 truncate font-mono text-[10px] text-zinc-600">{item.hash}</p>
								<p class="mt-1 text-[11px] text-zinc-500">
									{formatSize(item.size)} · {formatDate(item.uploaded_at)}
								</p>
							</div>
							{#if !active && !unavailable}
								<div class="flex shrink-0 gap-1">
									<Button
										variant="ghost"
										size="xs"
										class="text-zinc-300 hover:bg-white/8"
										loading={busyPackageId === item.id}
										onclick={() => activate(item)}
									>
										Activate
									</Button>
									<Button
										variant="ghost"
										size="icon-xs"
										class="text-red-300 hover:bg-red-500/10"
										disabled={Boolean(busyPackageId)}
										onclick={() => removePackage(item)}
										aria-label={`Delete package ${item.filename}`}
									>
										<Trash2 />
									</Button>
								</div>
							{/if}
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
</div>
