<script lang="ts">
	import { CircleAlert } from 'lucide-svelte';
	import LoginButton from './LoginButton.svelte';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';

	let { loginHref, error = '' }: { loginHref: string; error?: string } = $props();
</script>

<section class="mx-auto w-full max-w-xs">
	<h1 class="login-wordmark mb-6 text-center text-4xl font-bold tracking-[-0.065em] text-white sm:text-[2.7rem]">
		smoothie.
	</h1>

	{#if error}
		<Alert variant="error" class="mb-3">
			<CircleAlert />
			<AlertDescription>{error}</AlertDescription>
		</Alert>
	{/if}

	<LoginButton href={loginHref} />
</section>

<style>
	.login-wordmark {
		text-shadow: 1px 0 #f97316, -1px 0 #60a5fa;
	}
</style>
