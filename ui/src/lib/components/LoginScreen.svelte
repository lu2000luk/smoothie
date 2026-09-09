<script lang="ts">
	import { onMount } from 'svelte';
	import LoginCard from './LoginCard.svelte';
	import DashboardScreen from './DashboardScreen.svelte';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import { fetchMe, getSession, loginUrl, logout, type AuthUser } from '$lib/auth';

	let { error = '' }: { error?: string } = $props();

	let status: 'checking' | 'anonymous' | 'authenticated' = $state('checking');
	let user: AuthUser | null = $state(null);

	onMount(async () => {
		const token = getSession();
		if (!token) {
			status = 'anonymous';
			return;
		}
		try {
			user = await fetchMe(token);
			status = 'authenticated';
		} catch {
			status = 'anonymous';
		}
	});

	async function handleLogout() {
		const token = getSession();
		if (token) await logout(token);
		user = null;
		status = 'anonymous';
	}
</script>

{#if status === 'authenticated' && user}
	<DashboardScreen {user} onlogout={handleLogout} />
{:else}
	<div class="bg-background grid min-h-screen place-items-center px-4 py-12">
		{#if status === 'checking'}
			<Spinner class="size-8" />
		{:else}
			<LoginCard loginHref={loginUrl} {error} />
		{/if}
	</div>
{/if}
