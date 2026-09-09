<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import AppDetailScreen from '$lib/components/AppDetailScreen.svelte';
	import LoginCard from '$lib/components/LoginCard.svelte';
	import { Spinner } from '$lib/components/ui/spinner/index.js';
	import { fetchMe, getSession, loginUrl, logout, type AuthUser } from '$lib/auth';

	const appId = $derived(page.params.id ?? '');

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
	<AppDetailScreen {user} {appId} onlogout={handleLogout} />
{:else}
	<div class="login-shell flex min-h-screen items-center justify-center px-6 py-12">
		<main class="w-full">
			{#if status === 'checking'}
				<div class="flex justify-center">
					<Spinner class="size-8" />
				</div>
			{:else}
				<LoginCard loginHref={loginUrl} />
			{/if}
		</main>
	</div>
{/if}

<style>
	.login-shell {
		background: #171717;
	}
</style>
