<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { onMount } from "svelte"
	import type { ExtractKind, GameBrowserEntry, GameBrowserRequest, SearchFilter, SearchSort } from "$lib/bindings"
	import { Checkbox, Dropdown, Search, Accordion, AccordionItem, Button } from "carbon-components-svelte"
	import { event } from "$lib/utils"
	import { trackEvent } from "$lib/utils"
	import { help } from "$lib/helpray"
	import * as clipboard from "@tauri-apps/plugin-clipboard-manager"
	import { isEqual } from "lodash"
	import DocumentExport from "carbon-icons-svelte/lib/DocumentExport.svelte"

	export const elemID = "tree-" + Math.random().toString(36).replace(".", "")

	let tree: JSTree = $state(null!)

	function compareNodes(a: any, b: any) {
		if ((!(a.original ? a.original : a).folder && !(b.original ? b.original : b).folder) || ((a.original ? a.original : a).folder && (b.original ? b.original : b).folder)) {
			if (a?.original?.order && b?.original?.order && a?.original?.order !== b?.original?.order) {
				return a?.original?.order - b?.original?.order
			} else {
				return (a?.original?.chunk || a.text).localeCompare(b?.original?.chunk || b.text, undefined, {
					numeric: true,
					sensitivity: "base"
				}) > 0
					? 1
					: -1
			}
		} else {
			return (a.original ? a.original : a).folder ? -1 : 1
		}
	}

	function parsePath(path: string): {
		parents: string[]
		name: string
	} {
		if (path.startsWith("[")) {
			const [_, inner, params, filetype] = /^\[(.*)\](?:\((.*)\))?(\..*)?$/.exec(path)!
			const parsedInner = parsePath(inner)
			const type = filetype ? filetype.replace(/^\.pc_/, ".") : ""
			const name = `[${parsedInner.name}]` + (params ? `(${params})` : "") + type
			if (name.endsWith(`${type}]${type}`)) {
				return {
					parents: parsedInner.parents,
					name: name.match(/^\[(.*)\]\..*$/)![1]
				}
			} else if (name.match(new RegExp(`^\\[.*${RegExp.escape(type)}\\]\\(.*\\)${RegExp.escape(type)}$`))) {
				return {
					parents: parsedInner.parents,
					name: `[${name.match(/^\[(.*)\]\(.*\)..*$/)![1]}](${params})`
				}
			} else if ([".wwisebank", ".gfx", ".wes"].some((a) => name.endsWith(`]${a}`))) {
				return {
					parents: parsedInner.parents,
					name: name.match(/^\[(.*)\]\..*$/)![1]
				}
			} else if ([".class", ".aspect", ".brick", ".entity", ".entitytemplate"].some((ty) => name.endsWith(`${ty}].entitytype`))) {
				return {
					parents: parsedInner.parents,
					name: name.match(/^\[(.*)\]\..*$/)![1]
				}
			} else if ([".class", ".aspect", ".brick", ".entity", ".entitytemplate"].some((ty) => name.endsWith(`${ty}].entityblueprint`))) {
				return {
					parents: parsedInner.parents,
					name: name.match(/^\[(.*)\]\..*$/)![1] + " (blueprint)"
				}
			} else {
				return {
					parents: parsedInner.parents,
					name
				}
			}
		} else {
			const parents = path.split("/")

			return {
				parents: parents.slice(0, -1),
				name: parents.at(-1)!
			}
		}
	}

	onMount(async () => {
		jQuery("#" + elemID).jstree({
			core: {
				multiple: false,
				data: [],
				themes: {
					name: "default",
					dots: true,
					icons: true
				},
				check_callback: false,
				force_text: true,
				keyboard: {
					f2: () => {}
				}
			},
			search: {
				fuzzy: true,
				show_only_matches: true,
				close_opened_onclear: false
			},
			sort: function (a: any, b: any) {
				return compareNodes(this.get_node(a), this.get_node(b))
			},
			dnd: {
				copy: true
			},
			contextmenu: {
				select_node: false,
				items: (
					rightClickedNode: {
						id: string
						original: {
							folder: boolean
							path: string | null
							hint: string | null
							filetype: string
							extractKinds: [string, ExtractKind][]
						}
					},
					c: any
				) => {
					return rightClickedNode.original.folder
						? {}
						: {
								...(rightClickedNode.original.filetype === "TEMP"
									? {
											openInEditor: {
												separator_before: false,
												separator_after: false,
												_disabled: false,
												label: "Open in Editor",
												icon: "fa-regular fa-pen-to-square",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													trackEvent("Open QN entity in editor from game tree")

													await event({
														type: "tool",
														data: {
															type: "gameBrowser",
															data: {
																type: "openInEditor",
																data: { resource: selected_node.id }
															}
														}
													})
												}
											}
										}
									: {}),
								...(rightClickedNode.original.filetype === "REPO"
									? {
											openInEditor: {
												separator_before: false,
												separator_after: false,
												_disabled: false,
												label: "Open in Editor",
												icon: "fa-regular fa-pen-to-square",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													trackEvent("Open repository in editor from game tree")

													await event({
														type: "tool",
														data: {
															type: "gameBrowser",
															data: {
																type: "openInEditor",
																data: { resource: selected_node.id }
															}
														}
													})
												}
											}
										}
									: {}),
								...(rightClickedNode.id === "0057C2C3941115CA" || rightClickedNode.id === "0xAD6EC6AE7DBE39"
									? {
											openInEditor: {
												separator_before: false,
												separator_after: false,
												_disabled: false,
												label: "Open in Editor",
												icon: "fa-regular fa-pen-to-square",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													trackEvent("Open unlockables in editor from game tree")

													await event({
														type: "tool",
														data: {
															type: "gameBrowser",
															data: {
																type: "openInEditor",
																data: { resource: selected_node.id }
															}
														}
													})
												}
											}
										}
									: {}),
								...(rightClickedNode.original.extractKinds
									? {
											extract: {
												separator_before: true,
												separator_after: false,
												label: "Extract",
												icon: "fa-regular fa-save",
												action: false,
												submenu: Object.fromEntries(
													rightClickedNode.original.extractKinds.map(([name, kind], idx) => [
														idx.toString(),
														{
															separator_before: false,
															separator_after: false,
															_disabled: false,
															label: `${name[0].toUpperCase()}${name.slice(1)}`,
															icon: "fa-regular fa-save",
															action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
																const tree = jQuery.jstree!.reference(b.reference)
																const selected_node = tree.get_node(b.reference)

																trackEvent(`Extract ${name} from game tree`, {
																	hash: rightClickedNode.id,
																	filetype: rightClickedNode.original.filetype
																})

																await event({
																	type: "tool",
																	data: {
																		type: "gameBrowser",
																		data: {
																			type: "extract",
																			data: { resource: selected_node.id, kind }
																		}
																	}
																})
															}
														}
													])
												)
											}
										}
									: {}),
								copyHash: {
									separator_before: false,
									separator_after: false,
									_disabled: false,
									label: "Copy Hash",
									icon: "far fa-copy",
									action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
										trackEvent("Copy hash from game tree")

										const tree = jQuery.jstree!.reference(b.reference)
										const selected_node = tree.get_node(b.reference)

										await clipboard.writeText(selected_node.id)
									}
								},
								...(rightClickedNode.original.path
									? {
											copyPath: {
												separator_before: false,
												separator_after: false,
												_disabled: false,
												label: "Copy Path",
												icon: "far fa-copy",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													trackEvent("Copy path from game tree")

													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													await clipboard.writeText(selected_node.original.path)
												}
											}
										}
									: {}),
								...(rightClickedNode.original.hint
									? {
											copyhint: {
												separator_before: false,
												separator_after: false,
												_disabled: false,
												label: "Copy Hint",
												icon: "far fa-copy",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													trackEvent("Copy hint from game tree")

													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													await clipboard.writeText(selected_node.original.hint)
												}
											}
										}
									: {})
							}
				}
			},
			plugins: ["contextmenu", "sort", "dnd"]
		})

		tree = jQuery("#" + elemID).jstree()

		jQuery("#" + elemID).on("changed.jstree", async (_, { selected }: { selected: string[] }) => {
			if (selected.length) {
				const selected_node = tree.get_node(selected[0])
				if (selected_node && !selected_node.original.folder) {
					await event({
						type: "tool",
						data: {
							type: "gameBrowser",
							data: {
								type: "select",
								data: { resource: selected_node.id }
							}
						}
					})
				}

				tree.deselect_all(true)
			}
		})

		jQuery("#" + elemID).on("refresh.jstree", () => {
			if (tree.settings!.core.data.length < 500) {
				tree.open_all()
			}
		})
	})

	export async function handleRequest(request: GameBrowserRequest) {
		console.log("Game browser handling request", request)

		switch (request.type) {
			case "setEnabled":
				enabled = request.data.enabled
				if (!enabled) {
					tree.settings!.core.data = []
					tree.refresh()
				}
				break

			case "newTree":
				gameDescription = request.data.gameDescription
				entries = request.data.entries
				await refreshTree()
				break

			default:
				request satisfies never
				break
		}
	}

	async function refreshTree() {
		if (!tree) return

		tree.settings!.core.data = []

		const addedFolders: Record<string, any> = {}
		const addedPartitions: Record<string, any> = {}

		for (const entry of entries) {
			if (separatePartitions) {
				if (!addedPartitions[entry.partition[0]]) {
					tree.settings!.core.data.push({
						id: `partition-${entry.partition[0]}`,
						parent: "#",
						icon: "fa-solid fa-box",
						text: `${entry.partition[1]} (${entry.partition[0]})`,
						folder: true,
						path: null,
						filetype: null,
						chunk: entry.partition[0],
						order: entry.order
					})

					addedPartitions[entry.partition[0]] = tree.settings!.core.data.at(-1)!
				} else {
					addedPartitions[entry.partition[0]].order = Math.min(addedPartitions[entry.partition[0]].order || Infinity, entry.order || Infinity)
				}
			}

			if (entry.path) {
				const parsedPath = parsePath(entry.path)

				for (const pathSection of parsedPath.parents.map((_, i, arr) => arr.slice(0, i + 1).join("/"))) {
					const sectionID = separatePartitions ? `${entry.partition[0]}-${pathSection}` : pathSection
					if (!addedFolders[sectionID]) {
						tree.settings!.core.data.push({
							id: sectionID,
							parent: pathSection.split("/").slice(0, -1).join("/")
								? separatePartitions
									? `${entry.partition[0]}-${pathSection.split("/").slice(0, -1).join("/")}`
									: pathSection.split("/").slice(0, -1).join("/")
								: separatePartitions
									? `partition-${entry.partition[0]}`
									: "#",
							icon: "fa-regular fa-folder",
							text: pathSection.split("/").at(-1),
							folder: true,
							path: pathSection,
							filetype: null,
							order: entry.order
						})

						addedFolders[sectionID] = tree.settings!.core.data.at(-1)!
					} else {
						addedFolders[sectionID].order = Math.min(addedFolders[sectionID].order || Infinity, entry.order || Infinity)
					}
				}

				tree.settings!.core.data.push({
					id: entry.hash,
					parent: separatePartitions ? `${entry.partition[0]}-${parsedPath.parents.join("/")}` : parsedPath.parents.join("/"),
					icon: `${
						{
							TEMP: "fa-solid fa-cubes-stacked",
							ASET: "fa-regular fa-rectangle-list",
							CPPT: "fa-solid fa-diagram-project",
							TEXT: "fa-regular fa-image",
							TEXD: "fa-regular fa-image",
							MRTN: "fa-solid fa-person-running",
							FXAS: "fa-solid fa-person-running",
							ATMD: "fa-solid fa-person-running",
							UICT: "fa-regular fa-window-restore",
							PRIM: "fa-solid fa-shapes",
							WSGT: "fa-solid fa-volume-high",
							WSWT: "fa-solid fa-volume-high",
							WBNK: "fa-solid fa-volume-high",
							WWEV: "fa-solid fa-volume-high",
							WWFX: "fa-solid fa-explosion",
							WWEM: "fa-solid fa-music",
							WWES: "fa-solid fa-comments",
							SDEF: "fa-solid fa-comments",
							DLGE: "fa-solid fa-closed-captioning",
							LOCR: "fa-solid fa-language",
							RTLV: "fa-regular fa-closed-captioning",
							REPO: "fa-solid fa-code",
							JSON: "fa-solid fa-code",
							ORES: "fa-solid fa-code",
							GFXV: "fa-solid fa-film",
							LINE: "fa-solid fa-comment",
							CRMD: "fa-solid fa-people-group",
							NAVP: "fa-solid fa-route",
							AIRG: "fa-solid fa-route",
							AIBX: "fa-regular fa-user",
							AIBZ: "fa-regular fa-user",
							YSHP: "fa-solid fa-baseball-bat-ball",
							ALOC: "fa-solid fa-car-burst",
							TBLU: "fa-regular fa-square",
							CBLU: "fa-regular fa-square",
							ASEB: "fa-regular fa-square",
							UICB: "fa-regular fa-square",
							MATB: "fa-regular fa-square",
							WSWB: "fa-regular fa-square",
							DSWB: "fa-regular fa-square",
							ECPB: "fa-regular fa-square",
							WSGB: "fa-regular fa-square"
						}[entry.resourceType] || "fa-regular fa-file"
					}`,
					text: parsedPath.name,
					folder: false,
					path: entry.path,
					filetype: entry.resourceType,
					order: entry.order,
					extractKinds: entry.extractKinds
				})
			} else {
				tree.settings!.core.data.push({
					id: entry.hash,
					parent: separatePartitions ? `partition-${entry.partition[0]}` : "#",
					icon: `${
						{
							TEMP: "fa-solid fa-cubes-stacked",
							ASET: "fa-regular fa-rectangle-list",
							CPPT: "fa-solid fa-diagram-project",
							TEXT: "fa-regular fa-image",
							TEXD: "fa-regular fa-image",
							MRTN: "fa-solid fa-person-running",
							FXAS: "fa-solid fa-person-running",
							ATMD: "fa-solid fa-person-running",
							UICT: "fa-regular fa-window-restore",
							PRIM: "fa-solid fa-shapes",
							WSGT: "fa-solid fa-volume-high",
							WSWT: "fa-solid fa-volume-high",
							WBNK: "fa-solid fa-volume-high",
							WWEV: "fa-solid fa-volume-high",
							WWFX: "fa-solid fa-explosion",
							WWEM: "fa-solid fa-music",
							WWES: "fa-solid fa-comments",
							SDEF: "fa-solid fa-comments",
							DLGE: "fa-solid fa-closed-captioning",
							LOCR: "fa-solid fa-language",
							RTLV: "fa-regular fa-closed-captioning",
							REPO: "fa-solid fa-code",
							JSON: "fa-solid fa-code",
							ORES: "fa-solid fa-code",
							GFXV: "fa-solid fa-film",
							LINE: "fa-solid fa-comment",
							CRMD: "fa-solid fa-people-group",
							NAVP: "fa-solid fa-route",
							AIRG: "fa-solid fa-route",
							AIBX: "fa-regular fa-user",
							AIBZ: "fa-regular fa-user",
							AIBB: "fa-regular fa-user",
							YSHP: "fa-solid fa-baseball-bat-ball",
							ALOC: "fa-solid fa-car-burst",
							TBLU: "fa-regular fa-square",
							CBLU: "fa-regular fa-square",
							ASEB: "fa-regular fa-square",
							UICB: "fa-regular fa-square",
							MATB: "fa-regular fa-square",
							WSWB: "fa-regular fa-square",
							DSWB: "fa-regular fa-square",
							ECPB: "fa-regular fa-square",
							WSGB: "fa-regular fa-square"
						}[entry.resourceType] || "fa-regular fa-file"
					}`,
					text: entry.hint ? `${entry.hint} (${entry.hash}.${entry.resourceType})` : `${entry.hash}.${entry.resourceType}`,
					folder: false,
					path: null,
					hint: entry.hint || null,
					filetype: entry.resourceType,
					order: entry.order,
					extractKinds: entry.extractKinds
				})
			}
		}

		tree.refresh()
	}

	async function search() {
		if (searchQuery.length >= 3) {
			searchFeedback = ""
			await trackEvent("Search game files", {
				filter: searchFilter,
				sort: String(searchSort),
				separate_partitions: String(separatePartitions)
			})
			await event({
				type: "tool",
				data: {
					type: "gameBrowser",
					data: {
						type: "search",
						data: {
							query: searchQuery.toLowerCase(),
							filter: searchFilter,
							sort: {
								none: null,
								sizeAsc: ["Size", false],
								sizeDesc: ["Size", true]
							}[searchSort] as [SearchSort, boolean] | null
						}
					}
				}
			})
		} else if (searchQuery.length === 0) {
			searchFeedback = ""
			gameDescription = "Search for a game file above to get started"
			entries = []
			await refreshTree()
		} else {
			searchFeedback = "Search too broad"
			gameDescription = ""
			entries = []
			await refreshTree()
		}
	}

	async function searchInput(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		searchQuery = _event.target.value
		await search()
	}

	let enabled = $state(false)
	let gameDescription = $state("Search for a game file above to get started")
	let searchFeedback = $state("")
	let searchFilter: SearchFilter = $state("All")
	let searchSort: "none" | "sizeAsc" | "sizeDesc" = $state("none")
	let searchQuery = $state("")
	let separatePartitions = $state(false)
	let entries: GameBrowserEntry[] = $state([])

	$effect(() => {
		if (separatePartitions !== null) {
			;(async () => {
				await refreshTree()
				await trackEvent("Search game files", {
					filter: searchFilter,
					sort: String(searchSort),
					separate_partitions: String(separatePartitions)
				})
			})()
		}
	})
</script>

<div
	class="w-full h-full p-2 flex flex-col"
	use:help={{
		title: "Game content",
		description:
			"This panel lets you search the game files by hash, extension or path. Click a game resource to open an overview of it, or right-click to see more options. Some resources can also be dragged directly into an entity's tree."
	}}
>
	{#if !enabled}
		<div class="p-4">
			<p>You haven't selected a copy of the game to work with - go to the Settings tool to do that.</p>
		</div>
	{:else}
		<div class="pt-2 pb-1 px-2 leading-tight text-base">
			<div class="mb-4">
				<div
					use:help={{
						title: "Search query",
						description: 'You can separate multiple queries with spaces. For example, "agent47 default" matches only files containing both "agent47" and "default" in their path.'
					}}
				>
					<Search
						placeholder="Search game files..."
						size="lg"
						on:change={searchInput}
						on:clear={async () => {
							searchFeedback = ""
							gameDescription = ""
							entries = []
							await refreshTree()
							searchQuery = ""
						}}
						bind:value={searchQuery}
					/>
				</div>
				<div class="mt-2 flex gap-2">
					<Dropdown
						class="w-1/2 no-menu-spacing"
						size="sm"
						helperText="Filter"
						selectedId={searchFilter}
						items={[
							{ id: "All", text: "All" },
							{ id: "Templates", text: "Templates" },
							{ id: "Classes", text: "Classes" },
							{ id: "Models", text: "Models" },
							{ id: "Textures", text: "Textures" },
							{ id: "Sound", text: "Sound" }
						]}
						on:select={async ({ detail: { selectedId } }) => {
							searchFilter = selectedId
							await search()
						}}
					/>
					<Dropdown
						class="w-1/2 no-menu-spacing"
						size="sm"
						helperText="Sort"
						selectedId={searchSort}
						items={[
							{ id: "none", text: "Name" },
							{ id: "sizeAsc", text: "Size (Ascending)" },
							{ id: "sizeDesc", text: "Size (Descending)" }
						]}
						on:select={async ({ detail: { selectedId } }) => {
							searchSort = selectedId
							await search()
						}}
					/>
				</div>
			</div>
			<div
				class="mb-3"
				use:help={{
					title: "Separate tree by partition",
					description: "You can turn this on to group resources in the tree by the game partition, or chunk, they are found in."
				}}
			>
				<Checkbox labelText="Separate tree by partition" bind:checked={separatePartitions} />
			</div>
			<div>{searchFeedback}</div>
			<span class="text-neutral-400">{gameDescription}</span>
		</div>
	{/if}

	<div class="flex-grow overflow-y-auto">
		<div class="w-full h-full" id={elemID}></div>
	</div>

	{#if entries.some((a) => a.extractKinds.length)}
		<div class="mt-2 bg-neutral-800">
			<Accordion class="accordion-single w-full ">
				<AccordionItem title="Extract all">
					{#key entries}
						{const kinds: [string, ExtractKind][] = $state([])}
						{let usePaths = $state(false)}

						<div class="flex flex-col gap-2 max-h-[50vh] w-full overflow-y-auto pr-2 -mb-4">
							<div class="mb-2">
								<Checkbox labelText="Extract to paths" bind:checked={usePaths} />
							</div>

							{#each [...new Set(entries.flatMap((a) => a.resourceType))].sort((a, b) => entries.filter((c) => c.resourceType === b).length - entries.filter((c) => c.resourceType === a).length) as type}
								<div class="font-semibold">{type}</div>
								<div class="flex flex-wrap items-center gap-x-4">
									{#each entries
										.filter((a) => a.resourceType === type)
										.flatMap((a) => a.extractKinds)
										.filter((a, idx, arr) => arr.findIndex((b) => isEqual(a, b)) === idx) as [name, kind]}
										<div>
											<Checkbox
												labelText="{name[0].toUpperCase()}{name.slice(1)}"
												on:check={({ detail }) => {
													if (detail) {
														kinds.push([type, kind])
													} else {
														kinds.splice(
															kinds.findIndex((a) => a[0] === type && isEqual(a[1], kind)),
															1
														)
													}
												}}
											/>
										</div>
									{/each}
								</div>
							{/each}

							{const resourcesCount = $derived(entries.filter((a) => kinds.some((b) => b[0] === a.resourceType)).length)}
							<Button
								icon={DocumentExport}
								on:click={async () => {
									trackEvent("Mass extract", { files: resourcesCount })

									await event({
										type: "tool",
										data: {
											type: "gameBrowser",
											data: {
												type: "massExtract",
												data: { resources: entries.map((a) => a.hash), kinds, usePaths }
											}
										}
									})
								}}
							>
								Extract {resourcesCount} resource{resourcesCount !== 1 ? "s" : ""}
							</Button>
						</div>
					{/key}
				</AccordionItem>
			</Accordion>
		</div>
	{/if}
</div>

<style>
	:global(.accordion-single .bx--accordion__item) {
		border: none;
	}

	:global(.accordion-single .bx--accordion__heading) {
		min-height: 0;
		padding: 0.75rem 0 0.5rem 0;
		z-index: 0;
	}

	:global(.accordion-single .bx--accordion__content) {
		padding-right: 0;
	}
</style>
