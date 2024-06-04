<script lang="ts">
	import { computePosition, shift, flip, offset } from "@floating-ui/dom"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"

	const dispatch = createEventDispatcher()

	export let enabled = false

	let tooltipElem: HTMLDivElement
	let tooltipData: { title: string; description: string } | null = null

	let tooltipTop = visualViewport?.height || 0
	let tooltipLeft = visualViewport?.width || 0

	const handler: (evt: { clientX: number; clientY: number }) => void = async ({ clientX, clientY }) => {
		let element = document.elementFromPoint(clientX, clientY)
		let helpData = element?.getAttribute("data-helpray") || null

		while (helpData === null && element !== null) {
			element = element.parentElement
			helpData = element?.getAttribute("data-helpray") || null
		}

		if (helpData) {
			tooltipData = JSON.parse(helpData)
		} else {
			tooltipData = null
		}

		const virtualEl = {
			getBoundingClientRect() {
				return {
					width: 0,
					height: 0,
					x: clientX,
					y: clientY,
					left: clientX,
					right: clientX,
					top: clientY,
					bottom: clientY
				}
			}
		}

		;({ x: tooltipLeft, y: tooltipTop } = await computePosition(virtualEl, tooltipElem, {
			placement: "right-start",
			middleware: [offset({ mainAxis: 10, alignmentAxis: 10 }), flip(), shift()]
		}))
	}

	document.addEventListener("mousemove", handler)

	onDestroy(() => {
		document.removeEventListener("mousemove", handler)
	})

	$: if (enabled) {
		document.body.style.cursor = "help"
	} else {
		document.body.style.removeProperty("cursor")
	}
</script>

<svelte:window
	on:keydown={(e) => {
		if (e.key === "Escape") {
			enabled = false
		}
	}}
/>

<div class="absolute top-0 left-0 h-screen w-screen bg-opacity-20 bg-black pointer-events-none transition-opacity {enabled ? 'opacity-100' : 'opacity-0'}" style="z-index: 9999" />

<div bind:this={tooltipElem} class="absolute bg-[#505050] p-4 transition-opacity max-w-md" style="z-index: 99999; top: {tooltipTop}px; left: {tooltipLeft}px; opacity: {enabled ? '1' : '0'}">
	{#if tooltipData}
		<div class="font-bold mb-2">{tooltipData.title}</div>
		<div class="leading-snug">{tooltipData.description}</div>
	{:else}
		Hover over something to see help (press Escape to exit)
	{/if}
</div>
