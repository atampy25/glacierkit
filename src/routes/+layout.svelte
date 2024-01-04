<script lang="ts">
	import "../app.css"
	import "carbon-components-svelte/css/g90.css"
	import "font-awesome/css/font-awesome.min.css"

	import { appWindow } from "@tauri-apps/api/window"
	import { HeaderNavItem, HeaderNavMenu, SkipToContent } from "carbon-components-svelte"
	import { listen } from "@tauri-apps/api/event"
	import { onDestroy, onMount } from "svelte"
	import { flip } from "svelte/animate"
	import { fade } from "svelte/transition"

	let tasks: [string, string][] = []

	let destroyFunc = () => {}
	onDestroy(destroyFunc)

	onMount(async () => {
		const unlistenStartTask = await listen("start-task", ({ payload: task }: { payload: [string, string] }) => {
			tasks = [...tasks, task]
		})

		const unlistenFinishTask = await listen("finish-task", ({ payload: task }: { payload: string }) => {
			tasks = tasks.filter((a) => a[0] !== task)
		})

		destroyFunc = () => {
			unlistenStartTask()
			unlistenFinishTask()
		}
	})
</script>

<header data-tauri-drag-region class:bx--header={true}>
	<SkipToContent />
	<!-- svelte-ignore a11y-missing-attribute -->
	<a data-tauri-drag-region class:bx--header__name={true}>Deeznuts</a>

	<div data-tauri-drag-region class="pointer-events-none cursor-none w-full text-center text-neutral-400">Deeznuts</div>

	<div data-tauri-drag-region class="flex flex-row items-center justify-end text-white">
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.minimize}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M18 12H6" />
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.toggleMaximize}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M16.5 8.25V6a2.25 2.25 0 00-2.25-2.25H6A2.25 2.25 0 003.75 6v8.25A2.25 2.25 0 006 16.5h2.25m8.25-8.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-7.5A2.25 2.25 0 018.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 00-2.25 2.25v6"
				/>
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-red-600 active:bg-red-700" on:click={appWindow.close}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
			</svg>
		</div>
	</div>
</header>

<div class="w-full h-mid">
	<slot />
</div>

<div class="h-6 flex items-center gap-4 px-3 bg-neutral-600">
	{#each tasks as [id, task] (id)}
		<span transition:fade={{ duration: 100 }} animate:flip={{ duration: 250 }}>{task}</span>
	{/each}
</div>

<style>
	:global(.bx--header) {
		position: initial;
		display: flex;
		height: 3rem;
		align-items: center;
		border-bottom: 1px solid #393939;
		background-color: #161616;
	}

	.h-mid {
		height: calc(100vh - 3rem - 1.5rem);
	}

	:global(.bx--tooltip__trigger.bx--tooltip--right::after, .bx--tooltip__trigger.bx--tooltip--right .bx--assistive-text, .bx--tooltip__trigger.bx--tooltip--right + .bx--assistive-text) {
		background-color: #505050 !important;
		color: #f4f4f4 !important;
	}

	:global(.bx--tooltip__trigger.bx--tooltip--right::before) {
		border-color: rgba(0, 0, 0, 0) #505050 rgba(0, 0, 0, 0) rgba(0, 0, 0, 0) !important;
	}
</style>
