<script lang="ts">
	import { Card, CardPanel } from '$lib/components/ui/card/index.js';
	import AppIconDisplay from './AppIconDisplay.svelte';
	import type { App } from '$lib/apps';

	let { app }: { app: App } = $props();
</script>

<a href={`/apps/${encodeURIComponent(app.id)}`} class="block h-full">
	<Card class="h-full transition-colors hover:border-zinc-400 dark:hover:border-zinc-600">
		<CardPanel class="flex items-center gap-3">
			<AppIconDisplay kind={app.icon_kind} value={app.icon} />
			<span class="min-w-0 flex-1">
				<span class="block truncate text-sm font-semibold">{app.name}</span>
				<span class="block truncate font-mono text-xs text-zinc-500">{app.id}</span>
			</span>
		</CardPanel>
	</Card>
</a>
