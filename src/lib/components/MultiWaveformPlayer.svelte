<script lang="ts">
	import { Button } from "carbon-components-svelte"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import { v4 } from "uuid"
	import WaveSurfer from "wavesurfer.js"
	import Hover from "wavesurfer.js/dist/plugins/hover.esm.js"
	import Play from "carbon-icons-svelte/lib/Play.svelte"
	import Pause from "carbon-icons-svelte/lib/Pause.svelte"
	import SkipForward from "carbon-icons-svelte/lib/SkipForward.svelte"
	import SkipBack from "carbon-icons-svelte/lib/SkipBack.svelte"
	import Download from "carbon-icons-svelte/lib/Download.svelte"

	export let src: [string, string][]

	let container: HTMLDivElement
	let wavesurfer: WaveSurfer = null!

	let isPlaying = false

	let playerIdx = 0
	let displayedIndex: string;

	let current = 0
	let duration = 0

	const dispatch = createEventDispatcher()

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
			url: src[playerIdx][1],
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

			clearFunc.run = () => {
				wavesurfer?.destroy()
				clearInterval(interval)
			}
		})

		wavesurfer.on("finish", () => {
			isPlaying = false
		})
	})

	onDestroy(() => {
		clearFunc.run()
	})

	function commitIndexChange() {
		const newIndex = parseInt(displayedIndex, 10);

		if (isNaN(newIndex) || newIndex < 1 || newIndex > src.length) {
			// Invalid input, revert
			displayedIndex = (playerIdx + 1).toString();
		} else {
			// Valid input
			playerIdx = newIndex - 1;
			wavesurfer.load(src[playerIdx][1]);
			isPlaying = false;
		}
	}

	function handleKeyDown(e: KeyboardEvent) {
		const target = e.target as HTMLSpanElement;
		if (e.key === 'Enter') {
			e.preventDefault();
			commitIndexChange();
			target.blur();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			displayedIndex = (playerIdx + 1).toString(); // Revert
			target.blur();
		}
	}

	function handleBlur() {
		commitIndexChange();
	}

	// Ensure displayedIndexString is updated if playerIdx changes or on initialization
	$: displayedIndex = (playerIdx + 1).toString();
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
		<Button
			size="small"
			icon={SkipBack}
			iconDescription="Previous"
			on:click={() => {
				playerIdx = playerIdx - 1

				if (playerIdx < 0) {
					playerIdx = src.length - 1
				}

				wavesurfer.load(src[playerIdx][1])
				isPlaying = false
			}}
		/>
		<Button
			size="small"
			icon={SkipForward}
			iconDescription="Next"
			on:click={() => {
				playerIdx = playerIdx + 1

				if (playerIdx > src.length - 1) {
					playerIdx = 0
				}

				wavesurfer.load(src[playerIdx][1])
				isPlaying = false
			}}
		/>
		<Button
			size="small"
			icon={Download}
			iconDescription="Save"
			on:click={() => {
				dispatch("download", playerIdx)
			}}
		/>
		<span class="text-neutral-400">
			<span
				contenteditable="true"
				id="editable-audio-index"
				bind:textContent={displayedIndex}
				on:keydown={handleKeyDown}
				on:blur={handleBlur}
			></span> / {src.length}
		</span>
		<span class="text-neutral-400">{src[playerIdx][0]}</span>
		<span class="text-neutral-400">{secsToTime(current)} / {secsToTime(duration)}</span>
	</div>
{/if}

<style>
	#editable-audio-index:focus {
		background-color: #444;
		outline: none;
		padding: 2px 4px;
		border-radius: 3px;
	}
</style>
