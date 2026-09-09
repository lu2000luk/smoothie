<script lang="ts">
	import { ArrowRight } from 'lucide-svelte';
	import GithubLogo from './GithubLogo.svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let { href, class: className = '' }: { href: string; class?: string } = $props();
</script>

<Button {href} variant="outline" size="lg" class={`group h-12 w-full text-sm sm:h-12 ${className}`}>
	<GithubLogo size={20} class="size-5" />
	<span>Continue with GitHub</span>
	<ArrowRight class="transition-transform group-hover:translate-x-0.5" />
</Button>
