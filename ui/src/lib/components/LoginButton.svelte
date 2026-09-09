<script lang="ts">
	import { ArrowRight } from 'lucide-svelte';
	import GithubLogo from './GithubLogo.svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let { href }: { href: string } = $props();
</script>

<Button {href} size="lg" class="group w-full">
	<GithubLogo size={20} class="size-5" />
	<span>Continue with GitHub</span>
	<ArrowRight class="transition-transform group-hover:translate-x-0.5" />
</Button>
