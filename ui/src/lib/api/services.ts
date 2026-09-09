import { apiBase, getSession } from '$lib/auth';

export type ServiceState =
	'no-package' | 'uploading' | 'deploying' | 'running' | 'stopped' | 'failed';

export interface Deployment {
	id: string;
	service_id?: string;
	package_id?: string | null;
	status?: string | null;
	endpoint?: string | null;
	url?: string | null;
	host?: string | null;
	host_port?: number | null;
	created_at?: string;
	updated_at?: string;
	started_at?: string | null;
	finished_at?: string | null;
	error?: string | null;
}

export interface Service {
	id: string;
	app_id: string;
	owner_id: number;
	name: string;
	position_x: number;
	position_y: number;
	container_port: number | null;
	argv: string[] | string | null;
	active_package_id: string | null;
	active_deployment_id: string | null;
	package_ids: string[];
	created_at: string;
	updated_at: string;
	status?: string | null;
	deployment_status?: string | null;
	endpoint?: string | null;
	url?: string | null;
	deployment?: Deployment | null;
	active_deployment?: Deployment | null;
	current_deployment?: Deployment | null;
	deployments?: Deployment[];
}

export interface ServicePackage {
	id: string;
	filename: string;
	size: number;
	hash: string;
	status: string;
	uploaded_at: string;
	pruned_at: string | null;
}

export interface ServiceSettings {
	name?: string;
	container_port?: number | null;
	argv?: string[] | null;
	active_package_id?: string | null;
}

export interface ServiceActionResult {
	service: Service;
	package?: ServicePackage;
	deployment?: Deployment | null;
	error?: string | null;
}

function authHeaders(json = true): HeadersInit {
	const headers: Record<string, string> = {};
	const token = getSession();
	if (token) headers.Authorization = `Bearer ${token}`;
	if (json) headers['Content-Type'] = 'application/json';
	return headers;
}

async function handle<T>(response: Response): Promise<T> {
	if (!response.ok) {
		let message = `Request failed (${response.status})`;
		try {
			const data = (await response.json()) as { error?: string; message?: string };
			message = data.error ?? data.message ?? message;
		} catch {
			// The status still provides a useful fallback when the API has no JSON error body.
		}
		throw new Error(message);
	}
	if (response.status === 204) return undefined as T;
	return response.json() as Promise<T>;
}

function servicePath(appId: string, serviceId?: string): string {
	const base = `${apiBase}/apps/${encodeURIComponent(appId)}/services`;
	return serviceId ? `${base}/${encodeURIComponent(serviceId)}` : base;
}

type RawService = Partial<Service> &
	Pick<Service, 'id' | 'app_id' | 'name' | 'created_at' | 'updated_at'> & { owner?: number };
type RawPackage = Partial<ServicePackage> &
	Pick<ServicePackage, 'id'> & {
		sha256?: string;
		size_bytes?: number;
		blob_status?: string;
		created_at?: string;
		deleted_at?: string | null;
	};

function normalizeService(raw: RawService): Service {
	return {
		...raw,
		owner_id: raw.owner_id ?? raw.owner ?? 0,
		position_x: Math.round(Number(raw.position_x) || 0),
		position_y: Math.round(Number(raw.position_y) || 0),
		container_port: raw.container_port ?? null,
		argv: raw.argv ?? [],
		active_package_id: raw.active_package_id ?? null,
		active_deployment_id: raw.active_deployment_id ?? null,
		package_ids: raw.package_ids ?? []
	};
}

function normalizePackage(raw: RawPackage): ServicePackage {
	return {
		id: raw.id,
		filename: raw.filename ?? `${raw.id}.tar`,
		size: Number(raw.size ?? raw.size_bytes ?? 0),
		hash: raw.hash ?? raw.sha256 ?? '',
		status: raw.status ?? raw.blob_status ?? (raw.deleted_at ? 'deleted' : 'available'),
		uploaded_at: raw.uploaded_at ?? raw.created_at ?? '',
		pruned_at: raw.pruned_at ?? null
	};
}

function getService(data: RawService | { service: RawService }): Service {
	return normalizeService('service' in data ? data.service : data);
}

function normalizeAction(data: {
	service: RawService;
	package?: RawPackage;
	deployment?: Deployment | null;
	error?: string | null;
}): ServiceActionResult {
	return {
		service: normalizeService(data.service),
		package: data.package ? normalizePackage(data.package) : undefined,
		deployment: data.deployment,
		error: data.error
	};
}

export async function listServices(appId: string): Promise<Service[]> {
	const response = await fetch(servicePath(appId), { headers: authHeaders() });
	const data = await handle<{ services: RawService[] } | RawService[]>(response);
	return (Array.isArray(data) ? data : data.services).map(normalizeService);
}

export async function createService(
	appId: string,
	input: { name: string; position_x: number; position_y: number }
): Promise<Service> {
	const response = await fetch(servicePath(appId), {
		method: 'POST',
		headers: authHeaders(),
		body: JSON.stringify(input)
	});
	return getService(await handle<RawService | { service: RawService }>(response));
}

export async function updateService(
	appId: string,
	serviceId: string,
	settings: ServiceSettings
): Promise<Service> {
	const response = await fetch(servicePath(appId, serviceId), {
		method: 'PATCH',
		headers: authHeaders(),
		body: JSON.stringify(settings)
	});
	return getService(await handle<RawService | { service: RawService }>(response));
}

export async function updateServicePosition(
	appId: string,
	serviceId: string,
	position_x: number,
	position_y: number
): Promise<Service | null> {
	const response = await fetch(`${servicePath(appId, serviceId)}/position`, {
		method: 'PATCH',
		headers: authHeaders(),
		body: JSON.stringify({ position_x, position_y })
	});
	const data = await handle<RawService | { service: RawService } | { ok: boolean } | undefined>(
		response
	);
	if (!data || ('ok' in data && !('service' in data))) return null;
	return getService(data as RawService | { service: RawService });
}

export async function deleteService(appId: string, serviceId: string): Promise<void> {
	const response = await fetch(servicePath(appId, serviceId), {
		method: 'DELETE',
		headers: authHeaders()
	});
	await handle<unknown>(response);
}

export async function listServicePackages(
	appId: string,
	serviceId: string
): Promise<ServicePackage[]> {
	const response = await fetch(`${servicePath(appId, serviceId)}/packages`, {
		headers: authHeaders()
	});
	const data = await handle<{ packages: RawPackage[] } | RawPackage[]>(response);
	return (Array.isArray(data) ? data : data.packages).map(normalizePackage);
}

export async function uploadServicePackage(
	appId: string,
	serviceId: string,
	file: File
): Promise<ServiceActionResult> {
	const body = new FormData();
	body.append('package', file);
	const response = await fetch(`${servicePath(appId, serviceId)}/packages`, {
		method: 'POST',
		headers: authHeaders(false),
		body
	});
	const data = await handle<{
		service: RawService;
		package?: RawPackage;
		deployment?: Deployment | null;
		error?: string | null;
	}>(response);
	return normalizeAction(data);
}

export async function deleteServicePackage(
	appId: string,
	serviceId: string,
	packageId: string
): Promise<void> {
	const response = await fetch(
		`${servicePath(appId, serviceId)}/packages/${encodeURIComponent(packageId)}`,
		{ method: 'DELETE', headers: authHeaders() }
	);
	await handle<unknown>(response);
}

export async function activateServicePackage(
	appId: string,
	serviceId: string,
	packageId: string
): Promise<ServiceActionResult> {
	const response = await fetch(
		`${servicePath(appId, serviceId)}/packages/${encodeURIComponent(packageId)}/activate`,
		{ method: 'POST', headers: authHeaders() }
	);
	const data = await handle<{
		service: RawService;
		package?: RawPackage;
		deployment?: Deployment | null;
		error?: string | null;
	}>(response);
	return normalizeAction(data);
}

export async function startServiceDeployment(
	appId: string,
	serviceId: string
): Promise<ServiceActionResult> {
	const response = await fetch(`${servicePath(appId, serviceId)}/deployments/start`, {
		method: 'POST',
		headers: authHeaders()
	});
	return normalizeAction(
		await handle<{
			service: RawService;
			package?: RawPackage;
			deployment?: Deployment | null;
			error?: string | null;
		}>(response)
	);
}

export async function retryServiceDeployment(
	appId: string,
	serviceId: string,
	deploymentId: string
): Promise<ServiceActionResult> {
	const response = await fetch(
		`${servicePath(appId, serviceId)}/deployments/${encodeURIComponent(deploymentId)}/retry`,
		{ method: 'POST', headers: authHeaders() }
	);
	return normalizeAction(
		await handle<{
			service: RawService;
			package?: RawPackage;
			deployment?: Deployment | null;
			error?: string | null;
		}>(response)
	);
}

export async function stopServiceDeployment(
	appId: string,
	serviceId: string
): Promise<ServiceActionResult> {
	const response = await fetch(`${servicePath(appId, serviceId)}/deployments/stop`, {
		method: 'POST',
		headers: authHeaders()
	});
	return normalizeAction(
		await handle<{
			service: RawService;
			package?: RawPackage;
			deployment?: Deployment | null;
			error?: string | null;
		}>(response)
	);
}

export function currentDeployment(service: Service): Deployment | null {
	return (
		service.active_deployment ??
		service.current_deployment ??
		service.deployment ??
		service.deployments?.find((deployment) => deployment.id === service.active_deployment_id) ??
		service.deployments?.[0] ??
		null
	);
}

export function getServiceState(service: Service, override?: ServiceState): ServiceState {
	if (override) return override;
	const deployment = currentDeployment(service);
	const raw = (
		deployment?.status ??
		service.deployment_status ??
		service.status ??
		''
	).toLowerCase();
	if (['failed', 'error', 'crashed'].includes(raw)) return 'failed';
	if (['running', 'healthy', 'ready', 'active'].includes(raw)) return 'running';
	if (['deploying', 'building', 'starting', 'pending', 'queued'].includes(raw)) return 'deploying';
	if (['stopped', 'cancelled', 'canceled', 'exited', 'inactive'].includes(raw)) return 'stopped';
	if (!service.active_package_id && service.package_ids.length === 0) return 'no-package';
	if (service.active_deployment_id) return 'running';
	return 'stopped';
}
