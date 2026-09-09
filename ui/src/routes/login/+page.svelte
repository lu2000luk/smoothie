<script lang="ts">
	import { page } from '$app/state';
	import LoginScreen from '$lib/components/LoginScreen.svelte';

	const error = $derived(page.url.searchParams.get('error') ?? '');
</script>

<LoginScreen {error} />
