<script lang="ts">
	import jQuery from "jquery"
	import "jstree"
	import { onMount } from "svelte"
	import type { EntityTreeRequest, PastableTemplateCategory, RefProxy, SearchQuery } from "$lib/bindings"
	import { Modal } from "carbon-components-svelte"
	import { event, game } from "$lib/utils"
	import Filter from "carbon-icons-svelte/lib/Filter.svelte"
	import { changeReferenceToLocalEntity, genRandHex, getReferencedLocalEntity } from "./utils"
	import { trackEvent } from "$lib/utils"
	import HighlightMonaco from "./HighlightMonaco.svelte"
	import { v4 } from "uuid"
	import * as clipboard from "@tauri-apps/plugin-clipboard-manager"
	import SearchBar from "$lib/components/SearchBar.svelte"

	interface Props {
		editorID: string
	}

	let { editorID }: Props = $props()

	const elemID = "tree-" + Math.random().toString(36).replace(".", "")
	let tree: JSTree = $state(null!)

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

	// Gets around having to use JS for search
	let entitiesToShowOnSearch: Set<string> = new Set()

	let helpMenuOpen = $state(false)
	let helpMenuFactory = $state("")
	let helpMenuInputs: string[] = $state([])
	let helpMenuOutputs: string[] = $state([])
	let helpMenuDefaultPropertiesJSON = $state("")
	let helpMenuSubsets: string[] = $state([])

	let templates: PastableTemplateCategory[] = []

	let editorConnectionAvailable = false

	let addedEntities: string[] = []
	let removedEntities: [string, RefProxy | null, string, string, boolean][] = []
	let changedEntities: string[] = []

	let showDiff = false
	let diffTouchedEntities: string[] = []

	const zentityFactory = $derived(game.version === "fl" ? "[modules:/zentity.class].entitytype" : "[modules:/zentity.class].pc_entitytype")
	const zentityBlueprint = $derived(game.version === "fl" ? "[modules:/zentity.class].entityblueprint" : "[modules:/zentity.class].pc_entityblueprint")

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
				force_text: true,
				keyboard: {
					f2: () => {}
				}
			},
			search: {
				fuzzy: false,
				show_only_matches: true,
				close_opened_onclear: false,
				search_callback: (search: string, node: { id: string }) => entitiesToShowOnSearch.has(node.id)
			},
			sort: function (a: any, b: any) {
				return compareNodes(this.get_node(a), this.get_node(b))
			},
			contextmenu: {
				select_node: false,
				items: (b: { id: string }, c: any) => {
					return removedEntities.some((a) => a[0] === b.id)
						? {
								restore: {
									separator_before: false,
									separator_after: true,
									_disabled: false,
									label: !removedEntities.some((a) => a[0] === tree.get_node(b.id).parent) ? "Restore to Original" : "Restore Parent First",
									icon: !removedEntities.some((a) => a[0] === tree.get_node(b.id).parent) ? "fa fa-undo" : "fa fa-close",
									action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
										const tree = jQuery.jstree!.reference(b.reference)
										const selected_node = tree.get_node(b.reference)

										if (!removedEntities.some((a) => a[0] === tree.get_node(selected_node.id).parent)) {
											await event({
												type: "editor",
												data: {
													editor: editorID,
													data: {
														type: "entity",
														data: {
															type: "tree",
															data: {
																type: "restoreToOriginal",
																data: {
																	entityId: selected_node.id
																}
															}
														}
													}
												}
											})
										}
									}
								}
							}
						: {
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
												factory: zentityFactory,
												hasReverseParentRefs: false,
												parentRef: selected_node.id
											},
											getPositionOfNode(selected_node.id, "", false),
											function (a: any) {
												tree.edit(a, undefined, async (node, status, _c) => {
													if (!status || !node.text) {
														tree.delete_node(newEntityID)
														return
													}

													// Ensure parent gets reclassified as a folder if necessary
													selected_node.original.hasReverseParentRefs = true
													selected_node.original.folder = selected_node.original.factory === zentityFactory && selected_node.original.hasReverseParentRefs

													tree.set_icon(
														selected_node.id,
														selected_node.original.factory === zentityFactory && selected_node.original.hasReverseParentRefs
															? "fa-regular fa-folder"
															: icons.find((a) => selected_node.original.factory.includes(a[0]))
																? icons.find((a) => selected_node.original.factory.includes(a[0]))![1]
																: "fa-regular fa-file"
													)

													// If it's a folder it might move to the top
													tree.move_node(selected_node.id, selected_node.parent, getPositionOfNode(selected_node.parent, selected_node.text, selected_node.original.folder))

													await event({
														type: "editor",
														data: {
															editor: editorID,
															data: {
																type: "entity",
																data: {
																	type: "tree",
																	data: {
																		type: "create",
																		data: {
																			id: newEntityID,
																			content: {
																				parent: selected_node.id,
																				name: node.text,
																				factory: zentityFactory,
																				blueprint: zentityBlueprint
																			}
																		}
																	}
																}
															}
														}
													})

													// Add the entity ID to the displayed name
													tree.rename_node(node, `${node.text} (${node.id})`)
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

										// don't include entity ID in editing input
										tree.rename_node(selected_node, selected_node.text.split(" ").slice(0, -1).join(" "))

										tree.edit(selected_node, undefined, async (node, status, _cancelled) => {
											if (status) {
												tree.move_node(node, node.parent, getPositionOfNode(node.parent, node.text, node.original.folder))

												await event({
													type: "editor",
													data: {
														editor: editorID,
														data: {
															type: "entity",
															data: {
																type: "tree",
																data: {
																	type: "rename",
																	data: {
																		id: node.id,
																		newName: node.text
																	}
																}
															}
														}
													}
												})

												// re-add the entity ID
												tree.rename_node(node, `${node.text} (${node.id})`)
											} else {
												// re-add the entity ID
												tree.rename_node(node, `${node.text} (${node.id})`)
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

										if (selected_node.parent !== "#") {
											tree.get_node(selected_node.parent).original.hasReverseParentRefs = tree.settings!.core.data.some(
												(a: any) => a.parent === tree.get_node(selected_node.parent).id
											)
											tree.get_node(selected_node.parent).original.folder =
												tree.get_node(selected_node.parent).original.factory === zentityFactory && tree.get_node(selected_node.parent).original.hasReverseParentRefs

											// Reclassify parent as not folder if necessary
											tree.set_icon(
												selected_node.parent,
												tree.get_node(selected_node.parent).original.factory === zentityFactory && tree.get_node(selected_node.parent).original.hasReverseParentRefs
													? "fa-regular fa-folder"
													: icons.find((a) => tree.get_node(selected_node.parent).original.factory.includes(a[0]))
														? icons.find((a) => tree.get_node(selected_node.parent).original.factory.includes(a[0]))![1]
														: "fa-regular fa-file"
											)

											// If it's no longer a folder it might move down
											tree.move_node(
												selected_node.parent,
												tree.get_node(selected_node.parent).parent,
												getPositionOfNode(
													tree.get_node(selected_node.parent).parent,
													tree.get_node(selected_node.parent).text,
													tree.get_node(selected_node.parent).original.folder
												)
											)
										}

										await event({
											type: "editor",
											data: {
												editor: editorID,
												data: {
													type: "entity",
													data: {
														type: "tree",
														data: {
															type: "delete",
															data: {
																id: selected_node.id
															}
														}
													}
												}
											}
										})
									}
								},
								ccp: {
									separator_before: true,
									separator_after: false,
									label: "Clipboard",
									icon: "far fa-clipboard",
									action: false,
									submenu: {
										copy: {
											separator_before: false,
											separator_after: false,
											label: "Copy",
											icon: "far fa-copy",
											action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
												const tree = jQuery.jstree!.reference(b.reference)
												const selected_node = tree.get_node(b.reference)

												await event({
													type: "editor",
													data: {
														editor: editorID,
														data: {
															type: "entity",
															data: {
																type: "tree",
																data: {
																	type: "copy",
																	data: {
																		id: selected_node.id
																	}
																}
															}
														}
													}
												})
											}
										},
										paste: {
											separator_before: false,
											_disabled: false,
											separator_after: false,
											label: "Paste",
											icon: "far fa-paste",
											action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
												const tree = jQuery.jstree!.reference(b.reference)
												const selected_node = tree.get_node(b.reference)

												await event({
													type: "editor",
													data: {
														editor: editorID,
														data: {
															type: "entity",
															data: {
																type: "tree",
																data: {
																	type: "paste",
																	data: {
																		parentId: selected_node.id
																	}
																}
															}
														}
													}
												})
											}
										}
									}
								},
								templates: {
									separator_before: true,
									separator_after: false,
									label: "Templates",
									icon: "fa-solid fa-shapes",
									action: false,
									submenu: Object.fromEntries(
										templates.map((category) => [
											`templateCategory${category.name.replace(" ", "")}`,
											{
												separator_before: true,
												separator_after: false,
												label: category.name,
												icon: category.icon,
												action: false,
												submenu: Object.fromEntries(
													category.templates.map((template) => [
														`template${template.name.replace(" ", "")}`,
														{
															separator_before: false,
															_disabled: false,
															separator_after: false,
															label: template.name,
															icon: template.icon,
															action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
																trackEvent("Insert template", { template: template.name })

																const tree = jQuery.jstree!.reference(b.reference)
																const selected_node = tree.get_node(b.reference)

																await event({
																	type: "editor",
																	data: {
																		editor: editorID,
																		data: {
																			type: "entity",
																			data: {
																				type: "tree",
																				data: {
																					type: "useTemplate",
																					data: {
																						parentId: selected_node.id,
																						template: template.pasteData as unknown as any
																					}
																				}
																			}
																		}
																	}
																})
															}
														}
													])
												)
											}
										])
									)
								},
								...(editorConnectionAvailable
									? {
											editorConnection: {
												separator_before: true,
												separator_after: false,
												label: "Editor",
												icon: "fa-solid fa-right-left",
												action: false,
												submenu: {
													selectInEditor: {
														separator_before: false,
														separator_after: false,
														label: "Select in Editor",
														icon: "fas fa-highlighter",
														action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
															let d = tree.get_node(b.reference)

															trackEvent("Select in editor")

															await event({
																type: "editor",
																data: {
																	editor: editorID,
																	data: {
																		type: "entity",
																		data: {
																			type: "tree",
																			data: {
																				type: "selectEntityInEditor",
																				data: {
																					entityId: d.id
																				}
																			}
																		}
																	}
																}
															})
														}
													},
													moveToPlayerPosition: {
														separator_before: false,
														separator_after: false,
														label: "Move to Player Position",
														icon: "fa-solid fa-location-dot",
														action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
															let d = tree.get_node(b.reference)

															trackEvent("Move to player position")

															await event({
																type: "editor",
																data: {
																	editor: editorID,
																	data: {
																		type: "entity",
																		data: {
																			type: "tree",
																			data: {
																				type: "moveEntityToPlayer",
																				data: {
																					entityId: d.id
																				}
																			}
																		}
																	}
																}
															})
														}
													},
													rotateAsPlayer: {
														separator_before: false,
														separator_after: false,
														label: "Adjust Rotation to Player",
														icon: "fa-solid fa-location-dot",
														action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
															let d = tree.get_node(b.reference)

															trackEvent("Adjust rotation to player")

															await event({
																type: "editor",
																data: {
																	editor: editorID,
																	data: {
																		type: "entity",
																		data: {
																			type: "tree",
																			data: {
																				type: "rotateEntityAsPlayer",
																				data: {
																					entityId: d.id
																				}
																			}
																		}
																	}
																}
															})
														}
													},
													moveToCameraPosition: {
														separator_before: false,
														separator_after: false,
														label: "Move to Camera Position",
														icon: "fa-solid fa-location-dot",
														action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
															let d = tree.get_node(b.reference)

															trackEvent("Move to camera position")

															await event({
																type: "editor",
																data: {
																	editor: editorID,
																	data: {
																		type: "entity",
																		data: {
																			type: "tree",
																			data: {
																				type: "moveEntityToCamera",
																				data: {
																					entityId: d.id
																				}
																			}
																		}
																	}
																}
															})
														}
													},
													rotateAsCamera: {
														separator_before: false,
														separator_after: false,
														label: "Adjust Rotation to Camera",
														icon: "fa-solid fa-location-dot",
														action: async (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) => {
															let d = tree.get_node(b.reference)

															trackEvent("Adjust rotation to camera")

															await event({
																type: "editor",
																data: {
																	editor: editorID,
																	data: {
																		type: "entity",
																		data: {
																			type: "tree",
																			data: {
																				type: "rotateEntityAsCamera",
																				data: {
																					entityId: d.id
																				}
																			}
																		}
																	}
																}
															})
														}
													}
												}
											}
										}
									: {}),
								...(changedEntities.includes(b.id)
									? {
											revert: {
												separator_before: false,
												separator_after: true,
												_disabled: false,
												label: "Revert to Original",
												icon: "fa fa-undo",
												action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
													const tree = jQuery.jstree!.reference(b.reference)
													const selected_node = tree.get_node(b.reference)

													trackEvent("Revert changed entity to original")

													await event({
														type: "editor",
														data: {
															editor: editorID,
															data: {
																type: "entity",
																data: {
																	type: "tree",
																	data: {
																		type: "restoreToOriginal",
																		data: {
																			entityId: selected_node.id
																		}
																	}
																}
															}
														}
													})
												}
											}
										}
									: {}),
								copyID: {
									separator_before: false,
									separator_after: false,
									_disabled: false,
									label: "Copy ID",
									icon: "far fa-copy",
									action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
										const tree = jQuery.jstree!.reference(b.reference)
										const selected_node = tree.get_node(b.reference)

										await clipboard.writeText(selected_node.id)
									}
								},
								help: {
									separator_before: false,
									separator_after: false,
									_disabled: false,
									label: "Help",
									icon: "far fa-circle-question",
									action: async function (b: { reference: string | HTMLElement | JQuery<HTMLElement> }) {
										trackEvent("Show help menu")

										const tree = jQuery.jstree!.reference(b.reference)
										const selected_node = tree.get_node(b.reference)

										await event({
											type: "editor",
											data: {
												editor: editorID,
												data: {
													type: "entity",
													data: {
														type: "tree",
														data: {
															type: "showHelpMenu",
															data: {
																entityId: selected_node.id
															}
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
				if (selected_node && !removedEntities.some((a) => a[0] === selected_node.id)) {
					selectedNode = selected[0]

					await event({
						type: "editor",
						data: {
							editor: editorID,
							data: {
								type: "entity",
								data: {
									type: "tree",
									data: {
										type: "select",
										data: { id: selected[0] }
									}
								}
							}
						}
					})
				} else {
					fixSelection()
				}
			}
		})

		let currentlyCorrectingRemovedMovement = false

		jQuery("#" + elemID).on("move_node.jstree", async (_, { node, parent, old_parent }: { node: any; parent: string; old_parent: string }) => {
			if (removedEntities.some((a) => a[0] === node.id) || removedEntities.some((a) => a[0] === parent)) {
				if (currentlyCorrectingRemovedMovement) {
					currentlyCorrectingRemovedMovement = false
					return
				} else {
					currentlyCorrectingRemovedMovement = true
					tree.move_node(node, old_parent, getPositionOfNode(old_parent, node.text, node.original.folder))
					return
				}
			}

			if (parent !== old_parent) {
				tree.move_node(node, parent, getPositionOfNode(parent, node.text, node.original.folder))

				node.original.parentRef = parent !== "#" ? changeReferenceToLocalEntity(node.original.parentRef, parent) : null

				await event({
					type: "editor",
					data: {
						editor: editorID,
						data: {
							type: "entity",
							data: {
								type: "tree",
								data: {
									type: "reparent",
									data: { id: node.id, newParent: node.original.parentRef }
								}
							}
						}
					}
				})
			}
		})

		// Drag and drop from game browser
		jQuery("#" + elemID).on("copy_node.jstree", async (_, { node, original }: { node: { id: string; parent: string }; original: { id: string } }) => {
			trackEvent("Drag and drop from game browser to entity tree")

			tree.delete_node(node.id)

			await event({
				type: "editor",
				data: {
					editor: editorID,
					data: {
						type: "entity",
						data: {
							type: "tree",
							data: {
								type: "addGameBrowserItem",
								data: {
									parentId: node.parent,
									file: original.id
								}
							}
						}
					}
				}
			})
		})

		jQuery("#" + elemID).on("ready.jstree", () => {
			updateDiffing()
		})

		await event({
			type: "editor",
			data: {
				editor: editorID,
				data: {
					type: "entity",
					data: {
						type: "tree",
						data: {
							type: "initialise"
						}
					}
				}
			}
		})
	})

	export async function handleRequest(request: EntityTreeRequest) {
		console.log(`Tree for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "select":
				selectedNode = request.data.id
				tree.deselect_all(true)
				if (request.data.id) {
					tree.select_node(request.data.id)
				}
				tree.get_node(selectedNode, true)[0].scrollIntoView()
				break

			case "newTree":
				replaceTree(request.data.entities)
				break

			case "newItems":
				newItems(request.data.newEntities)
				break

			case "searchResults":
				entitiesToShowOnSearch = new Set(request.data.results)
				tree.search("dummy")
				break

			case "showHelpMenu":
				helpMenuFactory = request.data.factory
				helpMenuInputs = request.data.inputPins
				helpMenuOutputs = request.data.outputPins
				helpMenuDefaultPropertiesJSON = request.data.defaultPropertiesJson
				helpMenuSubsets = request.data.subsets
				helpMenuOpen = true
				break

			case "setTemplates":
				templates = request.data.templates
				break

			case "setEditorConnectionAvailable":
				editorConnectionAvailable = request.data.editorConnectionAvailable
				break

			case "setDiffInfo":
				addedEntities = request.data.new
				changedEntities = request.data.modified
				removedEntities = request.data.removed

				// May be called before tree is loaded
				try {
					updateDiffing()
				} catch (e) {
					console.log(e)
				}
				break

			case "setShowDiff":
				showDiff = request.data.showDiff
				updateDiffing()
				break

			default:
				request satisfies never
				break
		}
	}

	const icons = Object.entries({
		"[assembly:/templates/gameplay/ai2/actors.template?/npcactor.entitytemplate]": "fa-regular fa-user",
		"[assembly:/_pro/characters/templates/hero/agent47/agent47.template?/agent47_default.entitytemplate]": "fa-regular fa-user-circle",
		"[assembly:/_pro/design/levelflow.template?/herospawn.entitytemplate]": "fa-regular fa-user-circle",
		"[modules:/zglobaloutfitkit.class]": "fa fa-tshirt",
		"[modules:/zroomentity.class]": "fa fa-map-marker-alt",
		"[modules:/zboxvolumeentity.class]": "fa-regular fa-square",
		"[modules:/zsoundbankentity.class]": "fa fa-music",
		"[modules:/zcameraentity.class]": "fa fa-camera",
		"[modules:/zsequenceentity.class]": "fa fa-film",
		"[modules:/zhitmandamageovertime.class]": "fa fa-skull-crossbones",
		"[assembly:/_pro/design/templates/ld design assets/ld_helpers_generic.template?/mockup_commentbubble.entitytemplate]": "fa-regular fa-comment",
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
		zone: "fa-regular fa-square",
		fx: "fa fa-burst",
		timer: "fa-solid fa-hourglass", // Types

		"foliage/": "fa fa-seedling",
		"vehicles/": "fa fa-car-side",
		"environment/": "fa-regular fa-map",
		"logic/": "fa fa-cogs",
		"design/": "fa fa-swatchbook",
		"modules:/": "fa fa-project-diagram" // Paths
	})

	function replaceTree(nodes: [string, RefProxy | null, string, string, boolean][]) {
		tree.settings!.core.data = []

		for (const [entityID, parent, name, factory, hasReverseParentRefs] of nodes) {
			tree.settings!.core.data.push({
				id: entityID,
				parent: getReferencedLocalEntity(parent) || "#",
				icon:
					factory === zentityFactory && hasReverseParentRefs
						? "fa-regular fa-folder"
						: icons.find((a) => factory.includes(a[0]))
							? icons.find((a) => factory.includes(a[0]))![1]
							: "fa-regular fa-file",
				text: `${name} (${entityID})`,
				folder: factory === zentityFactory && hasReverseParentRefs,
				factory,
				hasReverseParentRefs,
				parentRef: parent
			})
		}

		tree.refresh()

		updateDiffing()
	}

	function newItems(nodes: [string, RefProxy | null, string, string, boolean][]) {
		let added = 0
		while (added < nodes.length) {
			for (const [entityID, parent, name, factory, hasReverseParentRefs] of nodes) {
				// We have to add the top-level entities first to ensure the tree responds appropriately
				if (!getReferencedLocalEntity(parent) || tree.get_node(getReferencedLocalEntity(parent) || "#")) {
					const existingNode = tree.get_node(entityID)

					if (existingNode) {
						tree.move_node(
							existingNode,
							getReferencedLocalEntity(parent) || "#",
							getPositionOfNode(getReferencedLocalEntity(parent) || "#", name, factory === zentityFactory && hasReverseParentRefs)
						)

						tree.rename_node(existingNode, `${name} (${entityID})`)

						tree.set_icon(
							existingNode,
							factory === zentityFactory && hasReverseParentRefs
								? "fa-regular fa-folder"
								: icons.find((a) => factory.includes(a[0]))
									? icons.find((a) => factory.includes(a[0]))![1]
									: "fa-regular fa-file"
						)

						existingNode.original.folder = factory === zentityFactory && hasReverseParentRefs
						existingNode.original.factory = factory
						existingNode.original.hasReverseParentRefs = hasReverseParentRefs
						existingNode.original.parentRef = parent

						if (getReferencedLocalEntity(parent)) {
							tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs = true
							tree.get_node(getReferencedLocalEntity(parent)).original.folder =
								tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs

							tree.set_icon(
								getReferencedLocalEntity(parent),
								tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs
									? "fa-regular fa-folder"
									: icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))
										? icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))![1]
										: "fa-regular fa-file"
							)

							tree.move_node(
								getReferencedLocalEntity(parent),
								tree.get_node(getReferencedLocalEntity(parent)).parent,
								getPositionOfNode(
									tree.get_node(getReferencedLocalEntity(parent)).parent,
									tree.get_node(getReferencedLocalEntity(parent)).text,
									tree.get_node(getReferencedLocalEntity(parent)).original.folder
								)
							)
						}
					} else {
						tree.create_node(
							getReferencedLocalEntity(parent) || "#",
							{
								id: entityID,
								parent: getReferencedLocalEntity(parent) || "#",
								icon:
									factory === zentityFactory && hasReverseParentRefs
										? "fa-regular fa-folder"
										: icons.find((a) => factory.includes(a[0]))
											? icons.find((a) => factory.includes(a[0]))![1]
											: "fa-regular fa-file",
								text: `${name} (${entityID})`,
								folder: factory === zentityFactory && hasReverseParentRefs,
								factory,
								hasReverseParentRefs,
								parentRef: parent
							},
							getPositionOfNode(getReferencedLocalEntity(parent) || "#", name, factory === zentityFactory && hasReverseParentRefs)
						)

						if (getReferencedLocalEntity(parent)) {
							tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs = true
							tree.get_node(getReferencedLocalEntity(parent)).original.folder =
								tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs

							tree.set_icon(
								getReferencedLocalEntity(parent),
								tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs
									? "fa-regular fa-folder"
									: icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))
										? icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))![1]
										: "fa-regular fa-file"
							)

							tree.move_node(
								getReferencedLocalEntity(parent),
								tree.get_node(getReferencedLocalEntity(parent)).parent,
								getPositionOfNode(
									tree.get_node(getReferencedLocalEntity(parent)).parent,
									tree.get_node(getReferencedLocalEntity(parent)).text,
									tree.get_node(getReferencedLocalEntity(parent)).original.folder
								)
							)
						}
					}

					added += 1
				}
			}
		}

		updateDiffing()
	}

	function updateDiffing() {
		for (const id of diffTouchedEntities) {
			if (tree.get_node(id)) {
				tree.get_node(id).li_attr.class = ""
				tree.get_node(id, true)[0]?.classList?.remove?.("item-new")
				tree.get_node(id, true)[0]?.classList?.remove?.("item-modified")
				tree.get_node(id, true)[0]?.classList?.remove?.("item-removed")
			}
		}

		diffTouchedEntities = []

		for (const entityID of removedEntities.map((a) => a[0])) {
			if (tree.get_node(entityID)) {
				tree.delete_node(entityID)
			}
		}

		if (showDiff) {
			for (const entityID of addedEntities) {
				tree.get_node(entityID).li_attr.class = "item-new"
				tree.get_node(entityID, true)[0]?.classList?.add?.("item-new")
				diffTouchedEntities.push(entityID)
			}

			for (const entityID of changedEntities) {
				tree.get_node(entityID).li_attr.class = "item-modified"
				tree.get_node(entityID, true)[0]?.classList?.add?.("item-modified")
				diffTouchedEntities.push(entityID)
			}

			let added = 0
			while (added < removedEntities.length) {
				for (const [entityID, parent, name, factory, hasReverseParentRefs] of removedEntities) {
					// We have to add the top-level entities first to ensure the tree responds appropriately
					if (!tree.get_node(entityID)) {
						if (!getReferencedLocalEntity(parent) || tree.get_node(getReferencedLocalEntity(parent) || "#")) {
							tree.create_node(
								getReferencedLocalEntity(parent) || "#",
								{
									id: entityID,
									parent: getReferencedLocalEntity(parent) || "#",
									icon:
										factory === zentityFactory && hasReverseParentRefs
											? "fa-regular fa-folder"
											: icons.find((a) => factory.includes(a[0]))
												? icons.find((a) => factory.includes(a[0]))![1]
												: "fa-regular fa-file",
									text: `${name} (${entityID})`,
									folder: factory === zentityFactory && hasReverseParentRefs,
									factory,
									hasReverseParentRefs,
									parentRef: parent,
									li_attr: {
										class: "item-removed"
									}
								},
								getPositionOfNode(getReferencedLocalEntity(parent) || "#", name, factory === zentityFactory && hasReverseParentRefs)
							)

							if (getReferencedLocalEntity(parent)) {
								tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs = true
								tree.get_node(getReferencedLocalEntity(parent)).original.folder =
									tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs

								tree.set_icon(
									getReferencedLocalEntity(parent),
									tree.get_node(getReferencedLocalEntity(parent)).original.factory === zentityFactory && tree.get_node(getReferencedLocalEntity(parent)).original.hasReverseParentRefs
										? "fa-regular fa-folder"
										: icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))
											? icons.find((a) => tree.get_node(getReferencedLocalEntity(parent)).original.factory.includes(a[0]))![1]
											: "fa-regular fa-file"
								)

								tree.move_node(
									getReferencedLocalEntity(parent),
									tree.get_node(getReferencedLocalEntity(parent)).parent,
									getPositionOfNode(
										tree.get_node(getReferencedLocalEntity(parent)).parent,
										tree.get_node(getReferencedLocalEntity(parent)).text,
										tree.get_node(getReferencedLocalEntity(parent)).original.folder
									)
								)
							}

							diffTouchedEntities.push(entityID)

							added += 1
						}
					}
				}
			}
		}
	}

	function fixSelection() {
		tree.deselect_all(true)

		if (!tree.get_node(selectedNode) || removedEntities.some((a) => a[0] === selectedNode)) {
			selectedNode = null
		}

		if (selectedNode) {
			tree.select_node(selectedNode, true)
		}
	}

	let selectedNode: string | null = $state(null)

	$effect(() => {
		if (selectedNode) {
			fixSelection()
		}
	})

	let searchQuery: SearchQuery = $state({ type: "simple", data: "" })
</script>

<SearchBar
	placeholder="Filter..."
	icon={Filter}
	size="lg"
	bind:query={searchQuery}
	on:change={async () => {
		if (searchQuery.data.length === 0) {
			tree.clear_search()
		} else {
			await event({
				type: "editor",
				data: {
					editor: editorID,
					data: {
						type: "entity",
						data: {
							type: "tree",
							data: {
								type: "search",
								data: {
									query: { type: searchQuery.type, data: searchQuery.data.toLowerCase() }
								}
							}
						}
					}
				}
			})
		}
	}}
	on:clear={() => {
		tree.clear_search()
	}}
/>
<div id={elemID} class="flex-grow overflow-auto"></div>

<Modal bind:open={helpMenuOpen} modalHeading="Help for {helpMenuFactory}" passiveModal>
	<div class="grid grid-cols-2 gap-4 h-[70vh]">
		<div class="flex flex-col gap-1">
			<h2>Default properties</h2>
			<div class="w-full flex-grow">
				<HighlightMonaco id={v4()} content={helpMenuDefaultPropertiesJSON} />
			</div>
		</div>
		<div>
			<h2>Pins</h2>

			{#if helpMenuInputs.length}
				<h3>Inputs</h3>
				<div class="mt-1 flex flex-row gap-2 flex-wrap">
					{#each helpMenuInputs as pin}
						<div class="inline-flex items-center p-2 rounded-sm bg-neutral-800">{pin}</div>
					{/each}
				</div>
			{/if}

			{#if helpMenuOutputs.length}
				<h3 class:mt-2={helpMenuInputs.length}>Outputs</h3>
				<div class="mt-1 flex flex-row gap-2 flex-wrap">
					{#each helpMenuOutputs as pin}
						<div class="inline-flex items-center p-2 rounded-sm bg-neutral-800">{pin}</div>
					{/each}
				</div>
			{/if}

			{#if helpMenuSubsets.length}
				<h2 class="mt-8">Subsets</h2>
				<div class="mt-1 flex flex-row gap-2 flex-wrap">
					{#each helpMenuSubsets as subset}
						<div class="inline-flex items-center p-2 rounded-sm bg-neutral-800">{subset}</div>
					{/each}
				</div>
			{/if}
		</div>
	</div>
</Modal>
