<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { onMount } from "svelte"
	import type { EntityTreeRequest, Ref } from "$lib/bindings-types"
	import { Search } from "carbon-components-svelte"
	import { event } from "$lib/utils"
	import Filter from "carbon-icons-svelte/lib/Filter.svelte"
	import { changeReferenceToLocalEntity, genRandHex, getReferencedLocalEntity } from "./utils"

	export let editorID: string

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
				items: (b: { id: string }, c: any) => {
					return {
						create: {
							separator_before: false,
							separator_after: true,
							_disabled: false,
							label: "Create Entity",
							icon: "fa fa-plus",
							action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
								const tree = jQuery.jstree!.reference(b.reference)
								const selected_node = tree.get_node(b.reference)

								const newEntityID = "cafe" + genRandHex(12)

								tree.create_node(
									selected_node,
									{
										id: newEntityID,
										parent: selected_node.id,
										icon: "fa fa-project-diagram",
										text: "",
										folder: false,
										parentRef: selected_node.id
									},
									getPositionOfNode(selected_node.id, "", false),
									function (a: any) {
										tree.edit(a, undefined, async (node, status, _c) => {
											if (!status || !node.text) {
												tree.delete_node(newEntityID)
												return
											}

											await event({
												type: "editor",
												data: {
													type: "entity",
													data: {
														type: "tree",
														data: {
															type: "create",
															data: {
																editor_id: editorID,
																id: newEntityID,
																content: {
																	parent: selected_node.id,
																	name: "New Entity",
																	factory: "[modules:/zentity.class].pc_entitytype",
																	blueprint: "[modules:/zentity.class].pc_entityblueprint"
																}
															}
														}
													}
												}
											})
										})
									}
								)
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

								tree.edit(selected_node, undefined, async (node, status, _cancelled) => {
									if (status) {
										tree.move_node(node, node.parent, getPositionOfNode(node.parent, node.text, node.original.folder))

										await event({
											type: "editor",
											data: {
												type: "entity",
												data: {
													type: "tree",
													data: {
														type: "rename",
														data: {
															editor_id: editorID,
															id: node.id,
															new_name: node.text
														}
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

								tree.is_selected(selected_node) ? tree.delete_node(tree.get_selected()) : tree.delete_node(selected_node)

								await event({
									type: "editor",
									data: {
										type: "entity",
										data: {
											type: "tree",
											data: {
												type: "delete",
												data: {
													editor_id: editorID,
													id: selected_node.id
												}
											}
										}
									}
								})
							}
						}
					}
				}
			},
			dnd: {
				copy: false
			},
			plugins: ["contextmenu", "dnd", "search", "sort"]
		})

		tree = jQuery("#" + elemID).jstree()

		jQuery("#" + elemID).on("changed.jstree", async (_, { selected }: { selected: string[] }) => {
			if (selected.length) {
				const selected_node = tree.get_node(selected[0])
				if (selected_node) {
					selectedNode = selected[0]

					await event({
						type: "editor",
						data: {
							type: "entity",
							data: {
								type: "tree",
								data: {
									type: "select",
									data: { editor_id: editorID, id: selected[0] }
								}
							}
						}
					})
				} else {
					fixSelection()
				}
			}
		})

		jQuery("#" + elemID).on("move_node.jstree", async (_, { node, parent }: { node: any; parent: string }) => {
			tree.move_node(node, parent, getPositionOfNode(parent, node.text, node.original.folder))

			node.original.parentRef = parent !== "#" ? changeReferenceToLocalEntity(node.original.parentRef, parent) : null

			await event({
				type: "editor",
				data: {
					type: "entity",
					data: {
						type: "tree",
						data: {
							type: "reparent",
							data: { editor_id: editorID, id: node.id, new_parent: node.original.parentRef }
						}
					}
				}
			})
		})

		await event({
			type: "editor",
			data: {
				type: "entity",
				data: {
					type: "tree",
					data: {
						type: "initialise",
						data: {
							editor_id: editorID
						}
					}
				}
			}
		})
	})

	export async function handleRequest(request: EntityTreeRequest) {
		console.log(`Tree for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "create":
				break

			case "delete":
				break

			case "rename":
				break

			case "select":
				selectedNode = request.data.id
				fixSelection()
				break

			case "newTree":
				await replaceTree(request.data.entities)
				break

			default:
				request satisfies never
				break
		}
	}

	const icons = Object.entries({
		"[assembly:/templates/gameplay/ai2/actors.template?/npcactor.entitytemplate].pc_entitytype": "fa-regular fa-user",
		"[assembly:/_pro/characters/templates/hero/agent47/agent47.template?/agent47_default.entitytemplate].pc_entitytype": "fa-regular fa-user-circle",
		"[assembly:/_pro/design/levelflow.template?/herospawn.entitytemplate].pc_entitytype": "fa-regular fa-user-circle",
		"[modules:/zglobaloutfitkit.class].pc_entitytype": "fa fa-tshirt",
		"[modules:/zroomentity.class].pc_entitytype": "fa fa-map-marker-alt",
		"[modules:/zboxvolumeentity.class].pc_entitytype": "fa-regular fa-square",
		"[modules:/zsoundbankentity.class].pc_entitytype": "fa fa-music",
		"[modules:/zcameraentity.class].pc_entitytype": "fa fa-camera",
		"[modules:/zsequenceentity.class].pc_entitytype": "fa fa-film",
		"[modules:/zhitmandamageovertime.class].pc_entitytype": "fa fa-skull-crossbones",
		"0059FBD4AEBCDED0": "fa-regular fa-comment", // Hashes

		"levelflow.template?/exit": "fa fa-sign-out-alt",
		zitem: "fa fa-wrench", // Specific

		blockup: "fa fa-cube",
		setpiece_container_body: "fa fa-box-open",
		setpiece_trap: "fa fa-skull-crossbones",
		animset: "fa fa-running",
		emitter: "fa fa-wifi",
		sender: "fa fa-wifi",
		event: "fa fa-location-arrow",
		death: "fa fa-skull",
		zone: "fa-regular fa-square", // Types
		fx: "fa fa-burst",

		"foliage/": "fa fa-seedling",
		"vehicles/": "fa fa-car-side",
		"environment/": "fa-regular fa-map",
		"logic/": "fa fa-cogs",
		"design/": "fa fa-swatchbook",
		"modules:/": "fa fa-project-diagram" // Paths
	})

	async function replaceTree(nodes: [string, Ref, string, string, boolean][]) {
		tree.settings!.core.data = []

		for (const [entityID, parent, name, factory, hasReverseParentRefs] of nodes) {
			tree.settings!.core.data.push({
				id: entityID,
				parent: getReferencedLocalEntity(parent) || "#",
				icon:
					factory == "[modules:/zentity.class].pc_entitytype" && hasReverseParentRefs
						? "fa-regular fa-folder"
						: icons.find((a) => factory.includes(a[0]))
							? icons.find((a) => factory.includes(a[0]))![1]
							: "fa-regular fa-file",
				text: name,
				folder: factory == "[modules:/zentity.class].pc_entitytype" && hasReverseParentRefs,
				parentRef: parent
			})
		}

		tree.refresh()
	}

	function searchInput(event: any) {
		const _event = event as { target: HTMLInputElement }
		tree.search(_event.target.value.toLowerCase())
	}

	function fixSelection() {
		tree.deselect_all(true)

		if (selectedNode) {
			tree.select_node(selectedNode, true)
		}
	}

	let selectedNode: string | null = null

	$: if (selectedNode) {
		fixSelection()
	}
</script>

<div class="mt-4 mb-2 mx-2 leading-tight text-base">
	<Search placeholder="Filter..." icon={Filter} size="lg" on:input={searchInput} />
</div>

<div class="h-full overflow-y-auto">
	<div id={elemID} />
</div>
