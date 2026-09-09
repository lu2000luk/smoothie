<script lang="ts">
	import { LogOut } from 'lucide-svelte';
	import type { AuthUser } from '$lib/auth';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Card,
		CardDescription,
		CardHeader,
		CardPanel,
		CardTitle
	} from '$lib/components/ui/card/index.js';

	let { user, onlogout }: { user: AuthUser; onlogout: () => void } = $props();

	const initials = $derived(user.login.slice(0, 2).toUpperCase());
</script>

<Card class="w-full max-w-sm">
	<CardHeader class="items-center text-center">
		<Avatar class="size-16">
			{#if user.avatar_url}
				<AvatarImage src={user.avatar_url} alt={user.login} />
			{:else}
				<AvatarFallback>{initials}</AvatarFallback>
			{/if}
		</Avatar>
		<CardTitle>{user.name ?? user.login}</CardTitle>
		<CardDescription>
			@{user.login}
			{#if user.email}<br />{user.email}{/if}
		</CardDescription>
	</CardHeader>

	<CardPanel>
		<Button variant="outline" class="w-full" onclick={onlogout}>
			<LogOut />
			<span>Log out</span>
		</Button>
	</CardPanel>
</Card>
