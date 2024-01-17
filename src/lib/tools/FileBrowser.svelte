<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import { join, sep } from "@tauri-apps/api/path"
	import type { FileBrowserRequest } from "$lib/bindings-types"
	import { Button, Search } from "carbon-components-svelte"
	import { event, showInFolder } from "$lib/utils"
	import { open } from "@tauri-apps/api/dialog"
	import FolderAdd from "carbon-icons-svelte/lib/FolderAdd.svelte"
	import { v4 } from "uuid"
	import Filter from "carbon-icons-svelte/lib/Filter.svelte"

	const elemID = "tree-" + Math.random().toString(36).replace(".", "")
	let tree: JSTree = null!

	function compareNodes(a: any, b: any) {
		if ((!(a.original ? a.original : a).folder && !(b.original ? b.original : b).folder) || ((a.original ? a.original : a).folder && (b.original ? b.original : b).folder)) {
			return a.text.localeCompare(b.text, undefined, { numeric: true, sensitivity: "base" }) > 0 ? 1 : -1
		} else {
			return (a.original ? a.original : a).folder ? -1 : 1
		}
	}

	function getPositionOfNode(parent: string, text: string, isFolder: boolean) {
		let indexOfNewNode = tree
			.settings!.core.data.filter((a: { parent: string }) => a.parent === parent)
			.sort(compareNodes)
			.findIndex((a: any) => compareNodes(a, { original: { folder: isFolder }, text, folder: isFolder }) > 0)

		if (indexOfNewNode === -1) {
			indexOfNewNode = "last"
		}

		return indexOfNewNode
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
				check_callback: true,
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
			contextmenu: {
				select_node: false,
				items: (rightClickedNode: { id: string; original: { folder: boolean } }, c: any) => {
					return {
						...(!rightClickedNode.original.folder
							? {}
							: {
									newfile: {
										separator_before: false,
										separator_after: true,
										_disabled: false,
										label: "New File",
										icon: "fa fa-plus",
										action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
											const tree = jQuery.jstree!.reference(b.reference)
											const selected_node = tree.get_node(b.reference)

											const id = v4()

											tree.create_node(
												selected_node,
												{
													id,
													parent: selected_node.id,
													icon: "fa-regular fa-file",
													text: "",
													folder: false
												},
												getPositionOfNode(selected_node.id, "", false),
												function (a: any) {
													tree.edit(a, undefined, async (node, status, _c) => {
														// Can't create entity.patch.json files
														if (!status || !node.text || node.text.endsWith(".entity.patch.json")) {
															tree.delete_node(id)
															return
														}

														const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.id], node.text)

														tree.set_icon(id, path.endsWith(".json") ? "fa-regular fa-pen-to-square" : "fa-regular fa-file")

														pathToID[path] = id

														await event({
															type: "tool",
															data: {
																type: "fileBrowser",
																data: {
																	type: "create",
																	data: {
																		path,
																		is_folder: false
																	}
																}
															}
														})
													})
												}
											)
										}
									},
									newfolder: {
										separator_before: false,
										separator_after: true,
										_disabled: false,
										label: "New Folder",
										icon: "fa fa-plus",
										action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
											const tree = jQuery.jstree!.reference(b.reference)
											const selected_node = tree.get_node(b.reference)

											const id = v4()

											tree.create_node(
												selected_node,
												{
													id,
													parent: selected_node.id,
													icon: "fa-regular fa-folder",
													text: "",
													folder: true
												},
												getPositionOfNode(selected_node.id, "", true),
												function (a: any) {
													tree.edit(a, undefined, async (node, status, _c) => {
														if (!status || !node.text) {
															tree.delete_node(id)
															return
														}

														const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.id], node.text)

														pathToID[path] = id

														await event({
															type: "tool",
															data: {
																type: "fileBrowser",
																data: {
																	type: "create",
																	data: {
																		path,
																		is_folder: true
																	}
																}
															}
														})
													})
												}
											)
										}
									}
								}),
						showinexplorer: {
							separator_before: false,
							separator_after: false,
							_disabled: false,
							label: "Show in Explorer",
							icon: "fa-regular fa-folder",
							action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
								const tree = jQuery.jstree!.reference(b.reference)
								const selected_node = tree.get_node(b.reference)

								const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.parent], selected_node.text)

								await showInFolder(path)
							}
						},
						rename: {
							separator_before: false,
							separator_after: false,
							_disabled: false,
							label: "Rename",
							icon: "fa-regular fa-pen-to-square",
							action: function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
								const tree = jQuery.jstree!.reference(b.reference)
								const selected_node = tree.get_node(b.reference)

								const oldName = selected_node.text

								tree.edit(selected_node, undefined, async (node, status, _cancelled) => {
									if (status) {
										tree.move_node(node, node.parent, getPositionOfNode(node.parent, node.text, node.original.folder))

										const oldPath = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[node.parent], oldName)
										const newPath = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[node.parent], node.text)

										delete pathToID[oldPath]
										pathToID[newPath] = node.id

										await event({
											type: "tool",
											data: {
												type: "fileBrowser",
												data: {
													type: "rename",
													data: {
														old_path: oldPath,
														new_path: newPath
													}
												}
											}
										})
									}
								})
							}
						},
						delete: {
							separator_before: false,
							separator_after: false,
							_disabled: false,
							label: "Delete",
							icon: "fa-regular fa-trash-can",
							action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
								const tree = jQuery.jstree!.reference(b.reference)
								const selected_node = tree.get_node(b.reference)

								const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.parent], selected_node.text)

								tree.is_selected(selected_node) ? tree.delete_node(tree.get_selected()) : tree.delete_node(selected_node)

								await event({
									type: "tool",
									data: {
										type: "fileBrowser",
										data: {
											type: "delete",
											data: path
										}
									}
								})
							}
						},
						...(!Object.fromEntries(Object.entries(pathToID).map(([a, b]) => [b, a]))[rightClickedNode.id].endsWith(".entity.json")
							? {}
							: {
									normaliseEntity: {
										separator_before: false,
										separator_after: false,
										_disabled: false,
										label: "Normalise",
										icon: "fa-solid fa-rotate",
										action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
											const tree = jQuery.jstree!.reference(b.reference)
											const selected_node = tree.get_node(b.reference)

											const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.parent], selected_node.text)

											await event({
												type: "tool",
												data: {
													type: "fileBrowser",
													data: {
														type: "normaliseQNFile",
														data: {
															path
														}
													}
												}
											})
										}
									}
								}),
						...(!Object.fromEntries(Object.entries(pathToID).map(([a, b]) => [b, a]))[rightClickedNode.id].endsWith(".entity.patch.json")
							? {}
							: {
									normalisePatch: {
										separator_before: false,
										separator_after: false,
										_disabled: false,
										label: "Normalise",
										icon: "fa-solid fa-rotate",
										action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
											const tree = jQuery.jstree!.reference(b.reference)
											const selected_node = tree.get_node(b.reference)

											const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.parent], selected_node.text)

											await event({
												type: "tool",
												data: {
													type: "fileBrowser",
													data: {
														type: "normaliseQNFile",
														data: {
															path
														}
													}
												}
											})
										}
									}
								})
					}
				}
			},
			dnd: {
				copy: false
			},
			plugins: ["contextmenu", "unique", "dnd", "search", "sort"]
		})

		tree = jQuery("#" + elemID).jstree()

		jQuery("#" + elemID).on("changed.jstree", async (_, { selected }: { selected: string[] }) => {
			if (selected.length) {
				const selected_node = tree.get_node(selected[0])
				if (selected_node && !selected_node.original.folder) {
					const path = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[selected_node.parent], selected_node.text)

					selectedFile = path

					await event({
						type: "tool",
						data: {
							type: "fileBrowser",
							data: {
								type: "select",
								data: path
							}
						}
					})
				} else {
					fixSelection()
				}
			}
		})

		jQuery("#" + elemID).on("move_node.jstree", async (_, { node, parent, old_parent }: { node: any; parent: string; old_parent: string }) => {
			if (parent !== old_parent && tree.get_node(old_parent).original?.folder) {
				if (tree.get_node(parent).original?.folder) {
					tree.move_node(node, parent, getPositionOfNode(parent, node.text, node.original.folder))

					const oldPath = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[old_parent], node.text)
					const newPath = await join(Object.fromEntries(Object.entries(pathToID).map((a) => [a[1], a[0]]))[parent], node.text)

					delete pathToID[oldPath]
					pathToID[newPath] = node.id

					await event({
						type: "tool",
						data: {
							type: "fileBrowser",
							data: {
								type: "rename",
								data: { old_path: oldPath, new_path: newPath }
							}
						}
					})
				} else {
					// Invalid move, reset the node
					tree.move_node(node, old_parent, getPositionOfNode(old_parent, node.text, node.original.folder))
				}
			}
		})
	})

	export async function handleRequest(request: FileBrowserRequest) {
		console.log("File browser handling request", request)

		switch (request.type) {
			case "create":
				if (!request.data.path.endsWith("project.json")) {
					pathToID[request.data.path] = v4()
					tree.create_node(
						pathToID[request.data.path.split(sep).slice(0, -1).join(sep)],
						{
							id: pathToID[request.data.path],
							parent: pathToID[request.data.path.split(sep).slice(0, -1).join(sep)],
							icon: `fa-regular fa-${request.data.is_folder ? "folder" : request.data.path.endsWith(".json") ? "pen-to-square" : "file"}`,
							text: request.data.path.split(sep).at(-1)!,
							folder: request.data.is_folder
						},
						getPositionOfNode(pathToID[request.data.path.split(sep).slice(0, -1).join(sep)], request.data.path.split(sep).at(-1)!, request.data.is_folder)
					)

					fixSelection()
				}
				break

			case "delete":
				if (!request.data.endsWith("project.json")) {
					tree.delete_node(pathToID[request.data])
					delete pathToID[request.data]
				}
				break

			case "rename":
				if (!(request.data.old_path.endsWith("project.json") || request.data.new_path.endsWith("project.json"))) {
					if (request.data.old_path.split(sep).slice(0, -1).join(sep) === request.data.new_path.split(sep).slice(0, -1).join(sep)) {
						tree.rename_node(tree.get_node(pathToID[request.data.old_path]), request.data.new_path.at(-1)!)
					} else {
						// To prevent uniqueness issues a UUID is generated and then the file is renamed back when it's done being moved
						tree.rename_node(tree.get_node(pathToID[request.data.old_path]), v4())
						tree.move_node(
							tree.get_node(pathToID[request.data.old_path]),
							pathToID[request.data.new_path.split(sep).slice(0, -1).join(sep)],
							getPositionOfNode(
								pathToID[request.data.new_path.split(sep).slice(0, -1).join(sep)],
								request.data.new_path.at(-1)!,
								tree.get_node(pathToID[request.data.old_path]).original.folder
							),
							() => {
								pathToID[request.data.new_path] = pathToID[request.data.old_path]
								delete pathToID[request.data.old_path]
								tree.rename_node(tree.get_node(pathToID[request.data.new_path]), request.data.new_path.at(-1)!)
							}
						)
					}
				}
				break

			case "select":
				selectedFile = request.data && pathToID[request.data] ? request.data : null
				fixSelection()
				break

			case "newTree":
				path = request.data.base_path
				await replaceTree(request.data.files)
				break

			default:
				request satisfies never
				break
		}
	}

	const pathToID: Record<string, string> = {}

	async function replaceTree(files: [string, boolean][]) {
		tree.settings!.core.data = []

		const rootNode = v4()

		const basePath = path

		tree.settings!.core.data.push({
			id: rootNode,
			parent: "#",
			icon: "fa-regular fa-folder",
			text: basePath.split(sep).at(-1),
			folder: true
		})

		pathToID[basePath] = rootNode

		for (const [path, isFolder] of files.filter((a) => a[0] !== basePath)) {
			if (path.trim() && !path.endsWith("project.json")) {
				const id = v4()

				tree.settings!.core.data.push({
					id,
					parent: pathToID[path.split(sep).slice(0, -1).join(sep)],
					icon: isFolder ? "fa-regular fa-folder" : path.endsWith(".json") ? "fa-regular fa-pen-to-square" : "fa-regular fa-file",
					text: path.split(sep).at(-1),
					folder: isFolder
				})

				pathToID[path] = id
			}
		}

		tree.refresh()
	}

	function searchInput(event: any) {
		const _event = event as { target: HTMLInputElement }
		tree.search(_event.target.value.toLowerCase())
	}

	function fixSelection() {
		tree.deselect_all(true)

		if (selectedFile) {
			tree.select_node(pathToID[selectedFile], true)
		}
	}

	let path = ""
	let selectedFile: string | null = null

	$: if (selectedFile) {
		fixSelection()
	}
</script>

<div class="w-full h-full p-2 flex flex-col">
	{#if !path}
		<div class="p-4">
			<p class="mb-4">You don't have a project loaded. Select a folder to get started!</p>
			<Button
				on:click={async () => {
					const path = await open({
						title: "Select the project folder",
						directory: true
					})

					if (typeof path === "string") {
						await event({ type: "global", data: { type: "loadWorkspace", data: path } })
					}
				}}
				icon={FolderAdd}
			>
				Select a project
			</Button>
		</div>
	{:else}
		<div class="pt-2 pb-1 px-2 leading-tight text-base">
			<div class="mb-4"><Search placeholder="Filter..." icon={Filter} size="lg" on:input={searchInput} /></div>
			<span class="text-neutral-400">{path}</span>
		</div>
	{/if}

	<div class="flex-grow overflow-y-auto">
		<div class="w-full h-full" id={elemID} />
	</div>
</div>
