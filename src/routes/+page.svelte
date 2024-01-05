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
	import { SvelteComponent, beforeUpdate, onDestroy } from "svelte"
	import { listen } from "@tauri-apps/api/event"
	import type { Request } from "$lib/bindings-types"
	import { Splitpanes, Pane } from "svelte-splitpanes"
	import Close from "carbon-icons-svelte/lib/Close.svelte"
	import Save from "carbon-icons-svelte/lib/Save.svelte"

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

	const toolComponents: Record<keyof typeof tools, { handleRequest: (request: any) => Promise<void> }> = ({} as unknown as null)!

	const editors = {} as const

	let tabs: {
		id: string
		name: string
		editor: SvelteComponent
		file: string | null
		unsaved: boolean
	}[] = []

	let activeTab: string | null = null

	let destroyFunc = () => {}
	onDestroy(destroyFunc)

	let hasListened = false

	beforeUpdate(async () => {
		if (!hasListened) {
			hasListened = true

			const unlisten = await listen("request", ({ payload: request }: { payload: Request }) => {
				switch (request.type) {
					case "tool":
						switch (request.data.type) {
							case "fileBrowser":
								void toolComponents.FileBrowser.handleRequest?.(request.data.data)
								break

							case "settings":
								void toolComponents.Settings.handleRequest?.(request.data.data)
								break

							case "gameBrowser":
								void toolComponents.GameBrowser.handleRequest?.(request.data.data)
								break

							default:
								request.data satisfies never
								break
						}
						break

					case "global":
						switch (request.data.type) {
							case "errorReport":
								// Handled by +layout.svelte
								break

							case "setWindowTitle":
								// Handled by +layout.svelte
								break

							default:
								request.data satisfies never
								break
						}
						break

					default:
						request satisfies never
						break
				}
			})

			destroyFunc = unlisten
		}
	})
</script>

<div class="h-full w-full flex">
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
	<Splitpanes class="w-full h-full" theme="">
		<Pane size={15}>
			<div class="w-full h-full bg-[#202020]">
				{#each typedEntries(tools) as [toolID, tool] (toolID)}
					<div class:hidden={selectedTool !== toolID}>
						<svelte:component this={tool.component} bind:this={toolComponents[toolID]} />
					</div>
				{/each}
			</div>
		</Pane>
		<Pane>
			{#if tabs.length}
				<div class="mt-2 mr-2 mb-2 min-h-10 bg-[#202020] flex flex-wrap">
					<div class="h-full pl-4 pr-1 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white border-b">
						Deeznuts
						<Button kind="ghost" size="field" icon={Save} iconDescription="Save" />
						<Button kind="ghost" size="field" icon={Close} iconDescription="Close" />
					</div>
					<div class="h-full pl-4 pr-1 flex gap-2 items-center justify-center cursor-pointer">
						Deeznuts 2
						<Button kind="ghost" size="field" icon={Close} iconDescription="Close" />
					</div>
				</div>
			{:else}
				<div class="h-full items-center justify-center">
					<h1>Welcome to Deeznuts</h1>
				</div>
			{/if}
		</Pane>
	</Splitpanes>
</div>
