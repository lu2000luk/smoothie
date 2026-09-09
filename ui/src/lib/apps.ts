import { apiBase, getSession } from './auth';

export type IconKind = 'emoji' | 'icon' | 'upload';

export interface App {
	id: string;
	owner_id: number;
	name: string;
	icon?: string | null;
	icon_kind?: IconKind | null;
	created_at: string;
	updated_at: string;
}

function authHeaders(): HeadersInit {
	const token = getSession();
	return token
		? { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }
		: { 'Content-Type': 'application/json' };
}

async function handle<T>(res: Response): Promise<T> {
	if (!res.ok) {
		let message = `request failed (${res.status})`;
		try {
			const data = await res.json();
			if (data?.error) message = data.error;
		} catch {
			/* ignore */
		}
		throw new Error(message);
	}
	return res.json() as Promise<T>;
}

export async function listApps(): Promise<App[]> {
	const res = await fetch(`${apiBase}/apps`, { headers: authHeaders() });
	const data = await handle<{ apps: App[] }>(res);
	return data.apps;
}

export async function getApp(id: string): Promise<App> {
	const res = await fetch(`${apiBase}/apps/${encodeURIComponent(id)}`, {
		headers: authHeaders()
	});
	const data = await handle<{ app: App }>(res);
	return data.app;
}

export async function createApp(
	name: string,
	icon: string | null,
	icon_kind: IconKind | null
): Promise<App> {
	const res = await fetch(`${apiBase}/apps`, {
		method: 'POST',
		headers: authHeaders(),
		body: JSON.stringify({ name, icon, icon_kind })
	});
	const data = await handle<{ app: App }>(res);
	return data.app;
}

export async function renameApp(
	id: string,
	name: string,
	icon?: string | null,
	icon_kind?: IconKind | null
): Promise<App> {
	const payload: Record<string, unknown> = { name };
	if (icon !== undefined) payload.icon = icon;
	if (icon_kind !== undefined) payload.icon_kind = icon_kind;
	const res = await fetch(`${apiBase}/apps/${encodeURIComponent(id)}`, {
		method: 'PATCH',
		headers: authHeaders(),
		body: JSON.stringify(payload)
	});
	const data = await handle<{ app: App }>(res);
	return data.app;
}

export async function deleteApp(id: string): Promise<void> {
	const res = await fetch(`${apiBase}/apps/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		headers: authHeaders()
	});
	await handle<{ ok: boolean }>(res);
}
