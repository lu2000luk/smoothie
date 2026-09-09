<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { fetchMe, setSession } from '$lib/auth';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Spinner } from '$lib/components/ui/spinner/index.js';

	let message = $state('Finishing sign in…');

	onMount(async () => {
		const params = page.url.searchParams;
		const session = params.get('session');
		const error = params.get('error');
		if (error) {
			await goto(`/login?error=${encodeURIComponent(error)}`);
			return;
		}
		if (!session) {
			await goto('/login?error=Sign+in+was+cancelled.+Please+try+again.');
			return;
		}
		setSession(session);
		try {
			await fetchMe(session);
			await goto('/');
		} catch {
			message = 'This sign-in link expired. Please try again.';
		}
	});
</script>

<div class="bg-background grid min-h-screen place-items-center px-4">
	<div class="flex w-full max-w-sm flex-col items-center gap-4 text-center">
		{#if message.startsWith('This sign-in')}
			<Alert variant="error">
				<AlertDescription class="text-center">{message}</AlertDescription>
			</Alert>
			<Button href="/login" variant="link">Back to login</Button>
		{:else}
			<Spinner class="size-8" />
			<p class="text-muted-foreground text-sm">{message}</p>
		{/if}
	</div>
</div>
