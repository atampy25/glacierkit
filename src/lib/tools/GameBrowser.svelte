<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { onMount } from "svelte"
	import type { GameBrowserEntry, GameBrowserRequest } from "$lib/bindings-types"
	import { Search } from "carbon-components-svelte"
	import { event } from "$lib/utils"
	import { clipboard } from "@tauri-apps/api"

	export const elemID = "tree-" + Math.random().toString(36).replace(".", "")

	export let tree: JSTree = null!

	function compareNodes(a: any, b: any) {
		if ((!(a.original ? a.original : a).folder && !(b.original ? b.original : b).folder) || ((a.original ? a.original : a).folder && (b.original ? b.original : b).folder)) {
			return a.text.localeCompare(b.text, undefined, { numeric: true, sensitivity: "base" }) > 0 ? 1 : -1
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
				force_text: true
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
				items: (rightClickedNode: { original: { path: string | null } }, c: any) => {
					return {
						copyHash: {
							separator_before: false,
							separator_after: false,
							_disabled: false,
							label: "Copy Hash",
							icon: "far fa-copy",
							action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
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
											const tree = jQuery.jstree!.reference(b.reference)
											const selected_node = tree.get_node(b.reference)

											await clipboard.writeText(selected_node.original.path)
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
				await replaceTree(request.data.entries)
				break

			default:
				request satisfies never
				break
		}
	}

	async function replaceTree(entries: GameBrowserEntry[]) {
		tree.settings!.core.data = []

		const addedFolders = new Set()

		for (const entry of entries) {
			if (entry.path) {
				const path = /\[(.*)\]\.pc_entity/.exec(entry.path)!

				for (const pathSection of path[1]
					.split("/")
					.map((_, ind, arr) => arr.slice(0, ind + 1).join("/"))
					.slice(0, -1)) {
					if (!addedFolders.has(pathSection)) {
						tree.settings!.core.data.push({
							id: pathSection,
							parent: pathSection.split("/").slice(0, -1).join("/") || "#",
							icon: "fa-regular fa-folder",
							text: pathSection.split("/").at(-1),
							folder: true,
							path: pathSection
						})

						addedFolders.add(pathSection)
					}
				}

				tree.settings!.core.data.push({
					id: entry.hash,
					parent: path[1].split("/").slice(0, -1).join("/"),
					icon: "fa-regular fa-file",
					text: path[1].split("/").at(-1),
					folder: false,
					path: entry.path
				})
			} else {
				tree.settings!.core.data.push({
					id: entry.hash,
					parent: "#",
					icon: "fa-regular fa-file",
					text: entry.hint ? `${entry.hint} (${entry.hash})` : entry.hash,
					folder: false,
					path: null
				})
			}
		}

		tree.refresh()
	}

	async function searchInput(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		if (_event.target.value.length >= 3) {
			searchFeedback = ""
			await event({
				type: "tool",
				data: {
					type: "gameBrowser",
					data: {
						type: "search",
						data: _event.target.value.toLowerCase()
					}
				}
			})
		} else if (_event.target.value.length >= 3) {
			searchFeedback = ""
			gameDescription = ""
			await replaceTree([])
		} else {
			searchFeedback = "Search too broad"
			gameDescription = ""
			await replaceTree([])
		}
	}

	let enabled = false
	let gameDescription = "Search for a game file above to get started"
	let searchFeedback = ""
</script>

<div class="w-full h-full p-2 flex flex-col">
	{#if !enabled}
		<div class="p-4">
			<p>You haven't selected a copy of the game to work with - go to the Settings tool to do that.</p>
		</div>
	{:else}
		<div class="pt-2 pb-1 px-2 leading-tight text-base">
			<div class="mb-4"
				><Search
					placeholder="Search game files..."
					size="lg"
					on:change={searchInput}
					on:clear={async () => {
						searchFeedback = ""
						gameDescription = ""
						await replaceTree([])
					}}
				/></div
			>
			<div>{searchFeedback}</div>
			<span class="text-neutral-400">{gameDescription}</span>
		</div>
	{/if}

	<div class="flex-grow overflow-y-auto">
		<div class="w-full h-full" id={elemID} />
	</div>
</div>
