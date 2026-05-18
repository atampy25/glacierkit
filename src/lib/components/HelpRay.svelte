<script lang="ts">
	import { computePosition, shift, flip, offset } from "@floating-ui/dom"
	import { throttle } from "lodash"

	let { enabled = $bindable() }: { enabled: boolean } = $props()

	let tooltipElem: HTMLDivElement | null = $state(null)
	let tooltipData: { title: string; description: string } | null = $state(null)

	let tooltipTop = $state(visualViewport?.height || 0)
	let tooltipLeft = $state(visualViewport?.width || 0)

	const handler: (evt: { clientX: number; clientY: number }) => void = ({ clientX, clientY }) => {
		if (!enabled) return

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

		if (tooltipElem) {
			void computePosition(virtualEl, tooltipElem, {
				placement: "right-start",
				middleware: [offset({ mainAxis: 10, alignmentAxis: 10 }), flip(), shift()]
			}).then(({ x, y }) => {
				tooltipLeft = x
				tooltipTop = y
			})
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === "Escape") {
			enabled = false
		}
	}}
/>

<svelte:document class:cursor-help={enabled} onmousemove={handler} />

<div class="absolute top-0 left-0 h-screen w-screen bg-opacity-20 bg-black pointer-events-none transition-opacity {enabled ? 'opacity-100' : 'opacity-0'}" style="z-index: 9999"></div>

<div bind:this={tooltipElem} class="absolute bg-[#505050] p-4 transition-opacity max-w-md" style="z-index: 99999; top: {tooltipTop}px; left: {tooltipLeft}px; opacity: {enabled ? '1' : '0'}">
	{#if tooltipData}
		<div class="font-bold mb-2">{tooltipData.title}</div>
		<div class="leading-snug">{tooltipData.description}</div>
	{:else}
		Hover over something to see help (press Escape to exit)
	{/if}
</div>
