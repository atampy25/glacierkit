<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { onMount } from "svelte"
	import type { GameBrowserEntry, GameBrowserRequest, SearchFilter } from "$lib/bindings-types"
	import { Checkbox, Dropdown, Search } from "carbon-components-svelte"
	import { event } from "$lib/utils"
	import { clipboard } from "@tauri-apps/api"
	import { trackEvent } from "@aptabase/tauri"
	import { help } from "$lib/helpray"

	export const elemID = "tree-" + Math.random().toString(36).replace(".", "")

	export let tree: JSTree = null!

	function compareNodes(a: any, b: any) {
		if ((!(a.original ? a.original : a).folder && !(b.original ? b.original : b).folder) || ((a.original ? a.original : a).folder && (b.original ? b.original : b).folder)) {
			return (a?.original?.chunk || a.text).localeCompare(b?.original?.chunk || b.text, undefined, { numeric: true, sensitivity: "base" }) > 0 ? 1 : -1
		} else {
			return (a.original ? a.original : a).folder ? -1 : 1
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
				items: (rightClickedNode: { id: string; original: { folder: boolean; path: string | null; hint: string | null; filetype: string } }, c: any) => {
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
																data: selected_node.id
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
																data: selected_node.id
															}
														}
													})
												}
											}
										}
									: {}),
								...(rightClickedNode.id === "0057C2C3941115CA"
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
																data: selected_node.id
															}
														}
													})
												}
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
								data: selected_node.id
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
				enabled = request.data
				if (!enabled) {
					tree.settings!.core.data = []
					tree.refresh()
				}
				break

			case "newTree":
				gameDescription = request.data.game_description
				entries = request.data.entries
				await refreshTree()
				break

			default:
				request satisfies never
				break
		}
	}

	async function refreshTree() {
		tree.settings!.core.data = []

		const addedFolders = new Set()
		const addedPartitions = new Set()

		for (const entry of entries) {
			if (separatePartitions && !addedPartitions.has(entry.partition[0])) {
				tree.settings!.core.data.push({
					id: `partition-${entry.partition[0]}`,
					parent: "#",
					icon: "fa-solid fa-box",
					text: `${entry.partition[1]} (${entry.partition[0]})`,
					folder: true,
					path: null,
					filetype: null,
					chunk: entry.partition[0]
				})

				addedPartitions.add(entry.partition[0])
			}

			if (entry.path) {
				const path = /\[(.*)\](?:\.pc_|\(.*\)\.pc_)/.exec(entry.path)![1]
				const params = /\[.*\]\((.*)\)\.pc_/.exec(entry.path)?.[1]
				const platformType = "." + /\[.*\](?:\.pc_|\(.*\)\.pc_)(.*)/.exec(entry.path)?.[1]

				for (const pathSection of path
					.split("/")
					.map((_, ind, arr) => arr.slice(0, ind + 1).join("/"))
					.slice(0, -1)) {
					if (!addedFolders.has(separatePartitions ? `${entry.partition[0]}-${pathSection}` : pathSection)) {
						tree.settings!.core.data.push({
							id: separatePartitions ? `${entry.partition[0]}-${pathSection}` : pathSection,
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
							filetype: null
						})

						addedFolders.add(separatePartitions ? `${entry.partition[0]}-${pathSection}` : pathSection)
					}
				}

				tree.settings!.core.data.push({
					id: entry.hash,
					parent: separatePartitions ? `${entry.partition[0]}-${path.split("/").slice(0, -1).join("/")}` : path.split("/").slice(0, -1).join("/"),
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
						}[entry.filetype] || "fa-regular fa-file"
					}`,
					text: (
						(params ? `[${path.split("/").at(-1)}](${params})` : path.split("/").at(-1)) +
						((platformType === ".entitytype" &&
							(path.endsWith(".class") ||
								path.endsWith(".aspect") ||
								path.endsWith(".brick") ||
								path.endsWith(".entity") ||
								path.endsWith(".entitytype") ||
								path.endsWith(".entitytemplate"))) ||
						platformType === ".wwisebank" ||
						platformType === ".gfx" ||
						platformType === ".wes" ||
						path.endsWith(platformType)
							? ""
							: platformType)
					).replace(/\.entityblueprint$/g, " (blueprint)"),
					folder: false,
					path: entry.path,
					filetype: entry.filetype
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
						}[entry.filetype] || "fa-regular fa-file"
					}`,
					text: entry.hint ? `${entry.hint} (${entry.hash}.${entry.filetype})` : `${entry.hash}.${entry.filetype}`,
					folder: false,
					path: null,
					hint: entry.hint || null,
					filetype: entry.filetype
				})
			}
		}

		tree.refresh()
	}

	async function searchInput(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		if (_event.target.value.length >= 3) {
			searchFeedback = ""
			await trackEvent("Search game files", { filter: searchFilter, separate_partitions: String(separatePartitions) })
			await event({
				type: "tool",
				data: {
					type: "gameBrowser",
					data: {
						type: "search",
						data: [_event.target.value.toLowerCase(), searchFilter]
					}
				}
			})
		} else if (_event.target.value.length === 0) {
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

	let enabled = false
	let gameDescription = "Search for a game file above to get started"
	let searchFeedback = ""
	let searchFilter: SearchFilter = "All"
	let searchQuery = ""
	let separatePartitions = false
	let entries: GameBrowserEntry[] = []

	$: separatePartitions,
		(async () => {
			await refreshTree()
			await trackEvent("Search game files", { filter: searchFilter, separate_partitions: String(separatePartitions) })
		})()
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
					class="flex gap-2"
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
					<Dropdown
						class="w-40 no-menu-spacing"
						bind:selectedId={searchFilter}
						items={[
							{ id: "All", text: "All" },
							{ id: "Templates", text: "Templates" },
							{ id: "Classes", text: "Classes" },
							{ id: "Models", text: "Models" },
							{ id: "Textures", text: "Textures" },
							{ id: "Sound", text: "Sound" }
						]}
						on:select={async ({ detail: { selectedId } }) => {
							if (searchQuery.length >= 3) {
								searchFeedback = ""
								await trackEvent("Search game files", { filter: selectedId, separate_partitions: String(separatePartitions) })
								await event({
									type: "tool",
									data: {
										type: "gameBrowser",
										data: {
											type: "search",
											data: [searchQuery.toLowerCase(), selectedId]
										}
									}
								})
							}
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
		<div class="w-full h-full" id={elemID} />
	</div>
</div>
