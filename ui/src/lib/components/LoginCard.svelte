<script lang="ts">
	import { CircleAlert } from 'lucide-svelte';
	import GithubLogo from './GithubLogo.svelte';
	import LoginButton from './LoginButton.svelte';
	import { Alert, AlertDescription } from '$lib/components/ui/alert/index.js';
	import {
		Card,
		CardDescription,
		CardFooter,
		CardHeader,
		CardPanel,
		CardTitle
	} from '$lib/components/ui/card/index.js';

	let { loginHref, error = '' }: { loginHref: string; error?: string } = $props();
</script>

<Card class="w-full max-w-sm">
	<CardHeader class="items-center text-center">
		<span
			class="bg-primary text-primary-foreground flex size-12 items-center justify-center rounded-xl"
		>
			<GithubLogo size={26} class="size-6" />
		</span>
		<CardTitle>Log in to Smoothie</CardTitle>
		<CardDescription>Use your GitHub account to continue.</CardDescription>
	</CardHeader>

	<CardPanel class="grid gap-4">
		{#if error}
			<Alert variant="error">
				<CircleAlert />
				<AlertDescription>{error}</AlertDescription>
			</Alert>
		{/if}
		<LoginButton href={loginHref} />
	</CardPanel>

	<CardFooter>
		<p class="text-muted-foreground w-full text-center text-xs">
			You will be redirected to GitHub to authorize.
		</p>
	</CardFooter>
</Card>
