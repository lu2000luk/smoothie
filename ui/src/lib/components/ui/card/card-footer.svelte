<script lang="ts">
	import { cn } from '$lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	let { class: className, children, ...restProps }: HTMLAttributes<HTMLDivElement> = $props();
</script>

<div
	data-slot="card-footer"
	class={cn(
		'flex items-center p-6 in-[[data-slot=card]:has(>[data-slot=card-panel])]:pt-4',
		className
	)}
	{...restProps}
>
	{@render children?.()}
</div>
