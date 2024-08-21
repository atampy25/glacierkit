<script lang="ts">
	import FileBrowser from "$lib/tools/FileBrowser.svelte"
	import Settings from "$lib/tools/Settings.svelte"
	import { event, typedEntries } from "$lib/utils"
	import Folders from "carbon-icons-svelte/lib/Folders.svelte"
	import Box from "carbon-icons-svelte/lib/Box.svelte"
	import SettingsIcon from "carbon-icons-svelte/lib/Settings.svelte"
	import GameBrowser from "$lib/tools/GameBrowser.svelte"
	import ToolButton from "$lib/components/ToolButton.svelte"
	import { Button, ToastNotification } from "carbon-components-svelte"
	import { beforeUpdate, onDestroy, onMount } from "svelte"
	import { listen } from "@tauri-apps/api/event"
	import type { Announcement, EditorType, Request } from "$lib/bindings-types"
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
	import Idea from "carbon-icons-svelte/lib/Idea.svelte"
	import ResourceOverviewEditor from "$lib/editors/resourceoverview/ResourceOverviewEditor.svelte"
	import { trackEvent } from "@aptabase/tauri"
	import RepositoryPatchEditor from "$lib/editors/repositorypatch/RepositoryPatchEditor.svelte"
	import UnlockablesPatchEditor from "$lib/editors/unlockablespatch/UnlockablesPatchEditor.svelte"
	import Search from "carbon-icons-svelte/lib/Search.svelte"
	import ContentSearch from "$lib/tools/ContentSearch.svelte"
	import ContentSearchResultsEditor from "$lib/editors/contentsearchresults/ContentSearchResultsEditor.svelte"
	import QuickStartEditor from "$lib/editors/quickstart/QuickStartEditor.svelte"
	import { open, confirm } from "@tauri-apps/api/dialog"
	import { help } from "$lib/helpray"
	import { v4 } from "uuid"

	const hints = [
		"You can switch between tabs with Ctrl-PageUp and Ctrl-PageDown (or Ctrl-Tab and Ctrl-Shift-Tab).",
		"You can save the active tab with Ctrl-S, or close it with Ctrl-W.",
		"Drag a valid template (entity, class, etc.) from the Game Content panel to the entity tree to create a new sub-entity with the given factory/blueprint.",
		"Generate random UUIDs, calculate game hashes from paths, or calculate localisation hashes from strings with the Text Tools panel.",
		"Pre-made templates for NPCs, logic entities and more are available from the Templates menu after right-clicking an entity.",
		"Want to turn an existing entity.json file into a patch, or vice-versa? Right-click the entry in the Files panel and press Convert to Entity/Patch.",
		"You can right-click a QuickEntity file and press Normalise to merge property overrides, sort keys and pad entity IDs to 16 characters.",
		"If you want to quickly edit a file externally, right-click it and press Show in Explorer to open its containing folder in Windows Explorer.",
		"The Help menu shows the default property values of a template or module, as well as the input and output pins it accepts. You can access it by right-clicking any entity in the tree.",
		"Changes that you make externally to your project folder are automatically synced to the Files panel in GlacierKit.",
		"Press Ctrl-Space to trigger intellisense in JSON editors - this can show you what properties are available and what their default values are.",
		'Right-click an entity\'s "factory" field and press "Open factory in new tab" or click on it and press F12 to quickly inspect its underlying template.',
		'You can visualise a ZCurve by right-clicking the property\'s name and pressing "Visualise curve".',
		'You can follow entity references by right-clicking the entity ID and pressing "Follow reference", or by pressing F12.',
		"Press F1 in any JSON editor to access the Command Palette, which lets you perform common operations like transforming text to lowercase/uppercase, or deleting duplicate lines.",
		'You can use Find and Replace in any JSON editor with Ctrl-H, or by pressing F1 and typing "replace".',
		"Right-click an entry in the Game Content panel to copy its hash or path.",
		"You can middle-click on a dependency in a Resource Overview to open it in a new tab.",
		"Convert between repository.json/unlockables.json files and JSON.patch.json files easily by right-clicking them in the Files panel.",
		"Many kinds of file can be previewed directly in the Resource Overview, including textures and sound files.",
		"Separate multiple search terms with spaces to find only items which match all of the search terms.",
		"Installing the ZHMModSDK and its Editor mod will allow you to modify entity positions and properties visually and in real time, and sync these with GlacierKit.",
		"You can open a file from outside of your current project by pressing CTRL-O."
	]

	let hint = hints[Math.floor(Math.random() * hints.length)]

	let announcements: Announcement[] = []

	let seenAnnouncements: string[] = []

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
		ContentSearch: {
			name: "Advanced search",
			icon: Search,
			component: ContentSearch
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

			case "QuickStart":
				return QuickStartEditor

			case "Text":
				return TextEditor

			case "QNEntity":
			case "QNPatch":
				return EntityEditor

			case "ResourceOverview":
				return ResourceOverviewEditor

			case "RepositoryPatch":
				return RepositoryPatchEditor

			case "UnlockablesPatch":
				return UnlockablesPatchEditor

			case "ContentSearchResults":
				return ContentSearchResultsEditor

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

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				type: "quickStart",
				data: {
					type: "create"
				}
			}
		})
	})

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

							case "contentSearch":
								void toolComponents.ContentSearch.handleRequest?.(request.data.data)
								break

							default:
								request.data satisfies never
								break
						}
						break

					case "global":
						switch (request.data.type) {
							case "errorReport":
							case "setWindowTitle":
							case "computeJSONPatchAndSave":
							case "requestLastPanicUpload":
							case "logUploadRejected":
								// Handled by +layout.svelte
								break

							case "initialiseDynamics":
								announcements = request.data.data.dynamics.announcements
								seenAnnouncements = request.data.data.seen_announcements
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
								const tabId = request.data.data

								const tabIndex = tabs.findIndex((a) => a.id === tabId)
								tabs = tabs.filter((a) => a.id !== tabId)

								if (activeTab === request.data.data) {
									activeTab = tabs.at(Math.max(tabIndex - 1, 0))?.id || null
								}

								void event({
									type: "global",
									data: {
										type: "selectTab",
										data: activeTab
									}
								})
								break

							case "renameTab":
								const id2 = request.data.data.id
								tabs.find((a) => a.id === id2)!.name = request.data.data.new_name
								tabs = tabs
								break

							default:
								request.data satisfies never
								break
						}
						break

					case "editor":
						switch (request.data.type) {
							case "quickStart":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
								break
							case "text":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
								break

							case "entity":
								void tabComponents[request.data.data.data.data.editor_id].handleRequest?.(request.data.data)
								break

							case "resourceOverview":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
								break

							case "repositoryPatch":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
								break

							case "unlockablesPatch":
								void tabComponents[request.data.data.data.id].handleRequest?.(request.data.data)
								break

							case "contentSearchResults":
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

<svelte:window
	use:shortcut={{
		key: "s",
		control: true,
		callback: async () => {
			if (activeTab && tabs.find((a) => a.id === activeTab)?.unsaved) {
				trackEvent("Save tab using CTRL-S")

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
				if (tabs.find((a) => a.id === activeTab)?.unsaved) {
					const result = await confirm("This tab has unsaved changes. Are you sure you want to close it?", {
						okLabel: "Don't Save",
						cancelLabel: "Cancel",
						title: "Unsaved changes",
						type: "warning"
					})

					if (!result) {
						return
					}
				}

				trackEvent("Close tab using CTRL-W")

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
			if (tabs.length) {
				trackEvent("Cycle tab forward using CTRL-Tab")

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
		}
	}}
	use:shortcut={{
		key: "PageDown",
		control: true,
		callback: async () => {
			if (tabs.length) {
				trackEvent("Cycle tab forward using CTRL-PgDown")

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
		}
	}}
	use:shortcut={{
		key: "Tab",
		control: true,
		shift: true,
		callback: async () => {
			if (tabs.length) {
				trackEvent("Cycle tab backward using CTRL-Shift-Tab")

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
		}
	}}
	use:shortcut={{
		key: "PageUp",
		control: true,
		callback: async () => {
			if (tabs.length) {
				trackEvent("Cycle tab backward using CTRL-PgUp")

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
		}
	}}
	use:shortcut={{
		key: "o",
		control: true,
		callback: async () => {
			trackEvent("Open file using CTRL-O")

			await event({
				type: "global",
				data: {
					type: "selectAndOpenFile"
				}
			})
		}
	}}
	use:shortcut={{
		key: "O",
		control: true,
		shift: true,
		callback: async () => {
			trackEvent("Load workspace using CTRL-SHIFT-O")

			const path = await open({
				title: "Select the project folder",
				directory: true
			})

			if (typeof path === "string") {
				await event({ type: "global", data: { type: "loadWorkspace", data: path } })
			}
		}
	}}
/>

<div class="h-full w-full flex">
	<div class="w-14 bg-neutral-900 flex flex-col" use:help={{ title: "Tools", description: "The left pane contains tools, which you can select here." }}>
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
								trackEvent("Reorder tabs by dragging")

								tabs.splice(evt.newIndex, 0, tabs.splice(evt.oldIndex, 1)[0])
								tabs = tabs
							}}
						>
							{#each tabs as tab (tab.id)}
								<div
									class="select-none h-full pl-4 pr-1 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
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
									on:mousedown={async (e) => {
										if (e.button === 1) {
											e.preventDefault()

											if (tab.unsaved) {
												const result = await confirm("This tab has unsaved changes. Are you sure you want to close it?", {
													okLabel: "Don't Save",
													cancelLabel: "Cancel",
													title: "Unsaved changes",
													type: "warning"
												})

												if (!result) {
													return
												}
											}

											trackEvent("Close tab using middle-click")

											await event({
												type: "global",
												data: {
													type: "removeTab",
													data: tab.id
												}
											})
										}
									}}
									use:help={{ title: "Tabs", description: "Each file or editor has an associated tab. You can reorder tabs by dragging." }}
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
													trackEvent("Save tab using button")

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
												if (tab.unsaved) {
													const result = await confirm("This tab has unsaved changes. Are you sure you want to close it?", {
														okLabel: "Don't Save",
														cancelLabel: "Cancel",
														title: "Unsaved changes",
														type: "warning"
													})

													if (!result) {
														return
													}
												}

												trackEvent("Close tab using button")

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
							<div class="flex-grow overflow-auto" class:hidden={activeTab !== tab.id}>
								<svelte:component this={tab.editor} bind:this={tabComponents[tab.id]} id={tab.id} />
							</div>
						{/each}
						{#if !activeTab}
							<div class="flex-grow flex items-center justify-center">
								<div class="text-center">
									<h1>Welcome to GlacierKit</h1>
									<p>Select a tab above to edit it here.</p>
								</div>
							</div>
						{/if}
					{:else}
						<div class="flex-grow flex items-center justify-center">
							<div class="text-center">
								<h1>GlacierKit</h1>
								{#if announcements.length}
									<div class="flex-col items-center -mb-4" use:help={{ title: "Announcements", description: "Any important announcements are displayed here." }}>
										{#each announcements as announcement (announcement.id)}
											{#if new Date().getTime() < (announcement.until || Number.MAX_VALUE) && !seenAnnouncements.includes(announcement.id)}
												<div class="text-left -mb-2 flex items-center justify-center -mr-4">
													<ToastNotification
														lowContrast
														kind={announcement.kind === "warning" ? "warning-alt" : announcement.kind}
														title={announcement.title}
														hideCloseButton={announcement.persistent}
														on:close={async () => {
															seenAnnouncements = [...seenAnnouncements, announcement.id]

															await event({
																type: "global",
																data: {
																	type: "setSeenAnnouncements",
																	data: seenAnnouncements
																}
															})
														}}
													>
														<div slot="subtitle">{@html announcement.description}</div>
													</ToastNotification>
												</div>
											{/if}
										{/each}
									</div>
								{/if}
								<!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
								<div
									class="mt-8 mx-16 flex gap-2 items-center text-neutral-300 cursor-pointer leading-snug"
									on:click={() => {
										hint = hints[(hints.indexOf(hint) + 1) % hints.length]
									}}
									use:help={{ title: "Hint", description: "A (hopefully) helpful hint." }}><Idea size={20} />{hint}</div
								>
							</div>
						</div>
					{/if}
				</div>
			</Pane>
		</Splitpanes>
	</div>
</div>
