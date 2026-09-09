export interface AuthUser {
	github_id: number;
	login: string;
	name: string | null;
	avatar_url: string | null;
	email: string | null;
	created_at: string;
	updated_at: string;
}

const SESSION_KEY = 'smoothie_session';

export const apiBase: string =
	(import.meta.env.VITE_API_URL as string | undefined) ?? 'http://localhost:3400';

export const loginUrl = `${apiBase}/auth/github`;

export function getSession(): string | null {
	if (typeof localStorage === 'undefined') return null;
	return localStorage.getItem(SESSION_KEY);
}

export function setSession(token: string): void {
	localStorage.setItem(SESSION_KEY, token);
}

export function clearSession(): void {
	localStorage.removeItem(SESSION_KEY);
}

export async function fetchMe(token: string): Promise<AuthUser> {
	const res = await fetch(`${apiBase}/auth/me`, {
		headers: { Authorization: `Bearer ${token}` }
	});
	if (!res.ok) throw new Error('session expired');
	const data = await res.json();
	return data.user as AuthUser;
}

export async function logout(token: string): Promise<void> {
	try {
		await fetch(`${apiBase}/auth/logout`, {
			method: 'POST',
			headers: { Authorization: `Bearer ${token}` }
		});
	} finally {
		clearSession();
	}
}
