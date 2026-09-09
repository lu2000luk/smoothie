<script lang="ts">
	import { ArrowRight } from 'lucide-svelte';
	import GithubLogo from './GithubLogo.svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let { href, class: className = '' }: { href: string; class?: string } = $props();
</script>

<Button {href} variant="outline" size="lg" class={`group h-10 w-full text-sm sm:h-10 ${className}`}>
	<GithubLogo size={20} class="size-5 pr-1" />
	<span>Continue with GitHub</span>
</Button>
