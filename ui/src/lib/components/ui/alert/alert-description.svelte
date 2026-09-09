<script lang="ts">
	import { cn } from '$lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	let { class: className, children, ...restProps }: HTMLAttributes<HTMLDivElement> = $props();
</script>

<div
	data-slot="alert-description"
	class={cn('flex flex-col gap-2.5 text-muted-foreground [svg~&]:col-start-2', className)}
	{...restProps}
>
	{@render children?.()}
</div>
