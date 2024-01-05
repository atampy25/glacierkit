<script lang="ts">
	import type { ComponentType, SvelteComponent } from "svelte"

	export let tooltip = ""
	export let icon: ComponentType<SvelteComponent>
	export let selected = false

	export let id = "tooltip-" + Math.random().toString(36)

	let hidden = false
</script>

<svelte:window
	on:keydown={({ key }) => {
		if (key === "Escape") {
			hidden = true
		}
	}}
/>

<div
	class="w-full justify-center py-4 bx--tooltip__trigger bx--tooltip--a11y bx--tooltip--right tool-button"
	class:tool-tab-selected={selected}
	class:bx--tooltip--hidden={hidden}
	on:click
	on:mouseover
	on:mouseenter
	on:mouseenter={() => {
		hidden = false
	}}
	on:mouseleave
	on:focus
	on:focus={() => {
		hidden = false
	}}
>
	<span {id} class="bx--assistive-text">{tooltip}</span>
	<svelte:component this={icon} size={22} />
</div>

<style>
	.tool-tab-selected {
		box-shadow: inset 2px 0px 0px 0px white;
	}

	:global(.bx--tooltip__trigger.tool-button.tool-tab-selected svg) {
		fill: white !important;
	}

	.bx--tooltip__trigger {
		display: flex !important;
		overflow: unset !important;
		align-items: unset !important;
		cursor: pointer !important;
		padding-top: 1rem !important;
		padding-bottom: 1rem !important;
		font-size: unset !important;
	}

	.bx--tooltip__trigger:hover {
		@apply bg-neutral-700;
	}
</style>
