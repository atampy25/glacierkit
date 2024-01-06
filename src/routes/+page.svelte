<script lang="ts">
	import FileBrowser from "$lib/tools/FileBrowser.svelte"
	import Settings from "$lib/tools/Settings.svelte"
	import { event, typedEntries } from "$lib/utils"
	import Folders from "carbon-icons-svelte/lib/Folders.svelte"
	import Box from "carbon-icons-svelte/lib/Box.svelte"
	import SettingsIcon from "carbon-icons-svelte/lib/Settings.svelte"
	import GameBrowser from "$lib/tools/GameBrowser.svelte"
	import ToolButton from "$lib/components/ToolButton.svelte"
	import { Button } from "carbon-components-svelte"
	import { SvelteComponent, beforeUpdate, onDestroy } from "svelte"
	import { listen } from "@tauri-apps/api/event"
	import type { EditorType, Request } from "$lib/bindings-types"
	import { Splitpanes, Pane } from "svelte-splitpanes"
	import Close from "carbon-icons-svelte/lib/Close.svelte"
	import Save from "carbon-icons-svelte/lib/Save.svelte"
	import NilEditor from "$lib/editors/nil/NilEditor.svelte"
	import TextEditor from "$lib/editors/text/TextEditor.svelte"
	import EntityEditor from "$lib/editors/entity/EntityEditor.svelte"

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

	function getEditor(editorType: EditorType) {
		switch (editorType.type) {
			case "nil":
				return NilEditor

			case "text":
				return TextEditor

			case "qnentity":
			case "qnpatch":
				return EntityEditor

			default:
				editorType satisfies never
				return NilEditor
		}
	}

	let tabs: {
		id: string
		name: string
		editor: ReturnType<typeof getEditor>
		file: string | null
		unsaved: boolean
	}[] = []

	const tabComponents: Record<string, { handleRequest: (request: any) => Promise<void> }> = {}

	let activeTab: string | null = null

	let destroyFunc = { run: () => {} }
	onDestroy(() => {
		destroyFunc.run()
	})

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

							case "createTab":
								tabs = [
									...tabs,
									{
										id: request.data.data.id,
										name: request.data.data.name,
										file: request.data.data.file,
										unsaved: false,
										editor: getEditor(request.data.data.editor_type)
									}
								]

								activeTab = request.data.data.id
								break

							case "setTabUnsaved":
								const id = request.data.data.id
								tabs.find((a) => a.id === id)!.unsaved = request.data.data.unsaved
								tabs = tabs
								break

							case "selectTab":
								activeTab = request.data.data
								break

							default:
								request.data satisfies never
								break
						}
						break

					case "editor":
						switch (request.data.type) {
							case "text":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
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

			destroyFunc.run = unlisten
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
					{#each tabs as tab (tab.id)}
						<div
							class="h-full pl-4 pr-1 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
							class:border-b={activeTab === tab.id}
							on:click={async () => {
								activeTab = tab.id

								await event({
									type: "global",
									data: {
										type: "selectTab",
										data: tab.id
									}
								})
							}}
						>
							{tab.name}
							{#if tab.unsaved}
								<Button
									kind="ghost"
									size="field"
									icon={Save}
									iconDescription="Save"
									on:click={async () => {
										if (tab.file) {
											await event({
												type: "global",
												data: {
													type: "saveTab",
													data: tab.id
												}
											})
										}
									}}
								/>
							{/if}
							<Button
								kind="ghost"
								size="field"
								icon={Close}
								iconDescription="Close"
								on:click={async () => {
									tabs = tabs.filter((a) => a.id !== tab.id)
									activeTab = null

									await event({
										type: "global",
										data: {
											type: "removeTab",
											data: tab.id
										}
									})
								}}
							/>
						</div>
					{/each}
				</div>
				{#each tabs as tab (tab.id)}
					<div class="h-full" class:hidden={activeTab !== tab.id}>
						<svelte:component this={tab.editor} bind:this={tabComponents[tab.id]} id={tab.id} />
					</div>
				{/each}
				{#if !activeTab}
					<div class="h-full flex items-center justify-center">
						<div class="text-center">
							<h1>Welcome to Deeznuts</h1>
							<p>Select a tab above to edit it here.</p>
						</div>
					</div>
				{/if}
			{:else}
				<div class="h-full flex items-center justify-center">
					<div class="text-center">
						<h1>Welcome to Deeznuts</h1>
						<p>You can start by selecting a project on the left.</p>
					</div>
				</div>
			{/if}
		</Pane>
	</Splitpanes>
</div>
