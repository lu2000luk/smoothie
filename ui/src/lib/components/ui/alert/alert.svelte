<script lang="ts" module>
	import { cn } from '$lib/utils.js';
	import { cva, type VariantProps } from 'class-variance-authority';

	export const alertVariants = cva(
		'relative grid w-full items-start gap-x-2 gap-y-0.5 rounded-xl border px-3.5 py-3 text-card-foreground text-sm has-[>svg]:has-data-[slot=alert-action]:grid-cols-[calc(var(--spacing)*4)_1fr_auto] has-[>svg]:grid-cols-[calc(var(--spacing)*4)_1fr] has-data-[slot=alert-action]:grid-cols-[1fr_auto] has-[>svg]:gap-x-2 [&>svg]:h-lh [&>svg]:w-4',
		{
			defaultVariants: {
				variant: 'default'
			},
			variants: {
				variant: {
					default: 'bg-transparent dark:bg-input/32 [&>svg]:text-muted-foreground',
					error: 'border-destructive/32 bg-destructive/4 [&>svg]:text-destructive',
					info: 'border-info/32 bg-info/4 [&>svg]:text-info',
					success: 'border-success/32 bg-success/4 [&>svg]:text-success',
					warning: 'border-warning/32 bg-warning/4 [&>svg]:text-warning'
				}
			}
		}
	);

	export type AlertVariant = VariantProps<typeof alertVariants>['variant'];
</script>

<script lang="ts">
	import type { HTMLAttributes } from 'svelte/elements';

	let {
		class: className,
		variant,
		children,
		...restProps
	}: HTMLAttributes<HTMLDivElement> & { variant?: AlertVariant } = $props();
</script>

<div
	data-slot="alert"
	role="alert"
	class={cn(alertVariants({ variant }), className)}
	{...restProps}
>
	{@render children?.()}
</div>
