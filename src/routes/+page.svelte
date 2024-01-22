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
	import { emit, listen } from "@tauri-apps/api/event"
	import type { EditorType, Request } from "$lib/bindings-types"
	import { Splitpanes, Pane } from "svelte-splitpanes"
	import Close from "carbon-icons-svelte/lib/Close.svelte"
	import Save from "carbon-icons-svelte/lib/Save.svelte"
	import NilEditor from "$lib/editors/nil/NilEditor.svelte"
	import TextEditor from "$lib/editors/text/TextEditor.svelte"
	import EntityEditor from "$lib/editors/entity/EntityEditor.svelte"
	import TextSelection from "carbon-icons-svelte/lib/TextSelection.svelte"
	import TextTransformer from "$lib/tools/TextTransformer.svelte"
	import { shortcut } from "$lib/shortcut"
	import { SortableList } from "@jhubbardsf/svelte-sortablejs"

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
		TextTransformer: {
			name: "Text tools",
			icon: TextSelection,
			component: TextTransformer
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
			case "Nil":
				return NilEditor

			case "Text":
				return TextEditor

			case "QNEntity":
			case "QNPatch":
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

							case "removeTab":
								const tabIndex = tabs.findIndex((a) => a.id === request.data.data)
								tabs = tabs.filter((a) => a.id !== request.data.data)
								activeTab = tabs.at(Math.max(tabIndex - 1, 0))?.id || null

								void event({
									type: "global",
									data: {
										type: "selectTab",
										data: activeTab
									}
								})
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

							case "entity":
								void tabComponents[request.data.data.data.data.editor_id].handleRequest?.(request.data.data)
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

<svelte:window
	use:shortcut={{
		key: "s",
		control: true,
		callback: async () => {
			if (activeTab && tabs.find((a) => a.id === activeTab)?.unsaved) {
				await event({
					type: "global",
					data: {
						type: "saveTab",
						data: activeTab
					}
				})
			}
		}
	}}
	use:shortcut={{
		key: "w",
		control: true,
		callback: async () => {
			if (activeTab) {
				await event({
					type: "global",
					data: {
						type: "removeTab",
						data: activeTab
					}
				})
			}
		}
	}}
	use:shortcut={{
		key: "Tab",
		control: true,
		callback: async () => {
			if (activeTab) {
				activeTab = tabs[(tabs.findIndex((a) => a.id === activeTab) + 1) % tabs.length].id
			} else {
				activeTab = tabs[0].id
			}

			await event({
				type: "global",
				data: {
					type: "selectTab",
					data: activeTab
				}
			})
		}
	}}
	use:shortcut={{
		key: "PageDown",
		control: true,
		callback: async () => {
			if (activeTab) {
				activeTab = tabs[(tabs.findIndex((a) => a.id === activeTab) + 1) % tabs.length].id
			} else {
				activeTab = tabs[0].id
			}

			await event({
				type: "global",
				data: {
					type: "selectTab",
					data: activeTab
				}
			})
		}
	}}
	use:shortcut={{
		key: "Tab",
		control: true,
		shift: true,
		callback: async () => {
			if (activeTab) {
				const nextLeft = tabs.findIndex((a) => a.id === activeTab) - 1
				activeTab = tabs[nextLeft >= 0 ? nextLeft : tabs.length - 1].id
			} else {
				activeTab = tabs[0].id
			}

			await event({
				type: "global",
				data: {
					type: "selectTab",
					data: activeTab
				}
			})
		}
	}}
	use:shortcut={{
		key: "PageUp",
		control: true,
		callback: async () => {
			if (activeTab) {
				const nextLeft = tabs.findIndex((a) => a.id === activeTab) - 1
				activeTab = tabs[nextLeft >= 0 ? nextLeft : tabs.length - 1].id
			} else {
				activeTab = tabs[0].id
			}

			await event({
				type: "global",
				data: {
					type: "selectTab",
					data: activeTab
				}
			})
		}
	}}
/>

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
	<div style="width: calc(100vw - 3.5rem)">
		<Splitpanes theme="">
			<Pane size={15}>
				<div class="w-full h-full bg-[#202020]">
					{#each typedEntries(tools) as [toolID, tool] (toolID)}
						<div class="w-full h-full" class:hidden={selectedTool !== toolID}>
							<svelte:component this={tool.component} bind:this={toolComponents[toolID]} />
						</div>
					{/each}
				</div>
			</Pane>
			<Pane class="h-full">
				<div class="h-full w-full flex flex-col py-2 pr-2 gap-2">
					{#if tabs.length}
						<SortableList
							class="h-10 flex-shrink-0 bg-[#202020] flex overflow-x-auto overflow-y-hidden"
							animation={150}
							forceFallback
							fallbackTolerance={5}
							onEnd={(evt) => {
								tabs.splice(evt.newIndex, 0, tabs.splice(evt.oldIndex, 1)[0])
								tabs = tabs
							}}
						>
							{#each tabs as tab (tab.id)}
								<div
									class="h-full pl-4 pr-1 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
									class:border-b={activeTab === tab.id}
									on:click|self={async () => {
										if (activeTab !== tab.id) {
											activeTab = tab.id

											await event({
												type: "global",
												data: {
													type: "selectTab",
													data: tab.id
												}
											})
										}
									}}
								>
									{tab.name}
									<div class="flex">
										{#if tab.unsaved}
											<Button
												kind="ghost"
												size="field"
												icon={Save}
												iconDescription="Save (CTRL-S)"
												on:click={async () => {
													await event({
														type: "global",
														data: {
															type: "saveTab",
															data: tab.id
														}
													})
												}}
											/>
										{/if}
										<Button
											kind="ghost"
											size="field"
											icon={Close}
											iconDescription="Close (CTRL-W)"
											on:click={async () => {
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
								</div>
							{/each}
						</SortableList>
						{#each tabs as tab (tab.id)}
							<div class="flex-grow" class:hidden={activeTab !== tab.id}>
								<svelte:component this={tab.editor} bind:this={tabComponents[tab.id]} id={tab.id} />
							</div>
						{/each}
						{#if !activeTab}
							<div class="flex-grow flex items-center justify-center">
								<div class="text-center">
									<h1>Welcome to Deeznuts</h1>
									<p>Select a tab above to edit it here.</p>
								</div>
							</div>
						{/if}
					{:else}
						<div class="flex-grow flex items-center justify-center">
							<div class="text-center">
								<h1>Welcome to Deeznuts</h1>
								<p>You can start by selecting a project on the left.</p>
							</div>
						</div>
					{/if}
				</div>
			</Pane>
		</Splitpanes>
	</div>
</div>
