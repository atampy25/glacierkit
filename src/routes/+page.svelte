<script lang="ts">
	import FileBrowser from "$lib/tools/FileBrowser.svelte"
	import Settings from "$lib/tools/Settings.svelte"
	import { typedEntries } from "$lib/utils"
	import Folders from "carbon-icons-svelte/lib/Folders.svelte"
	import Box from "carbon-icons-svelte/lib/Box.svelte"
	import SettingsIcon from "carbon-icons-svelte/lib/Settings.svelte"
	import GameBrowser from "$lib/tools/GameBrowser.svelte"
	import ToolButton from "$lib/components/ToolButton.svelte"
	import { Button } from "carbon-components-svelte"
	import { event } from "$lib/bindings"

	const tools = {
		FileBrowser: {
			name: "Files",
			icon: Folders,
			component: FileBrowser
		},
		GameBrowser: {
			name: "Game content",
			icon: Box,
			component: GameBrowser
		},
		Settings: {
			name: "Settings",
			icon: SettingsIcon,
			component: Settings
		}
	} as const

	let selectedTool: keyof typeof tools = "FileBrowser"
</script>

<div class="w-full h-full flex">
	<div class="w-14 bg-neutral-900 flex flex-col">
		{#each typedEntries(tools) as [toolID, tool] (toolID)}
			<ToolButton
				icon={tool.icon}
				on:click={() => {
					selectedTool = toolID
				}}
				selected={selectedTool === toolID}
				tooltip={tool.name}
			/>
		{/each}
	</div>
	<div class="w-80 bg-[#202020]">
		{#each typedEntries(tools) as [toolID, tool] (toolID)}
			<div class:hidden={selectedTool !== toolID}>
				<svelte:component this={tool.component}></svelte:component>
			</div>
		{/each}
	</div>
	<div class="flex-grow p-8">
		<Button
			on:click={async () => {
				await event({ type: "global", data: { type: "workspaceLoaded", data: { path: "" } } })
			}}>Load a workspace</Button
		>
	</div>
</div>
