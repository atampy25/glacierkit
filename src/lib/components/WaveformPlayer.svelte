<script lang="ts">
	import { Button } from "carbon-components-svelte"
	import { onDestroy, onMount } from "svelte"
	import { v4 } from "uuid"
	import WaveSurfer from "wavesurfer.js"
	import Hover from "wavesurfer.js/dist/plugins/hover.esm.js"
	import Play from "carbon-icons-svelte/lib/Play.svelte"
	import Pause from "carbon-icons-svelte/lib/Pause.svelte"

	export let src: string

	let container: HTMLDivElement
	let wavesurfer: WaveSurfer = null!

	let isPlaying = false

	let current = 0
	let duration = 0

	const clearFunc = { run: () => {} }

	function secsToTime(seconds: number) {
		var date = new Date(0)
		date.setSeconds(seconds)
		return date.toISOString().substring(14, 19)
	}

	onMount(() => {
		wavesurfer = WaveSurfer.create({
			container,
			waveColor: "#dddddd",
			progressColor: "#aaaaaa",
			url: src,
			plugins: [
				Hover.create({
					lineColor: "#888888",
					lineWidth: 1,
					labelBackground: "#555",
					labelColor: "#fff",
					labelSize: "11px"
				})
			]
		})

		wavesurfer.on("ready", () => {
			duration = wavesurfer.getDuration()

			const interval = setInterval(() => {
				current = wavesurfer.getCurrentTime()
			}, 200)

			clearFunc.run = () => clearInterval(interval)
		})

		wavesurfer.on("finish", () => {
			isPlaying = false
		})
	})

	onDestroy(() => {
		clearFunc.run()
	})
</script>

<div class="mb-2" bind:this={container}></div>
{#if wavesurfer}
	<div class="flex flex-wrap gap-4 items-center">
		<Button
			size="small"
			icon={isPlaying ? Pause : Play}
			iconDescription={isPlaying ? "Pause" : "Play"}
			on:click={() => {
				isPlaying = !isPlaying
				wavesurfer.playPause()
			}}
		/>
		<span class="text-neutral-400">{secsToTime(current)} / {secsToTime(duration)}</span>
	</div>
{/if}
