<script lang="ts">
	import type { EntityMetadataRequest, SubType } from "$lib/bindings"
	import { event } from "$lib/utils"
	import { TextInput, Dropdown } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import ListEditor from "$lib/components/ListEditor.svelte"
	import md5 from "md5"
	import { help } from "$lib/helpray"

	export let editorID: string

	let factory = ""
	let blueprint = ""
	let rootEntity = ""
	let subType: SubType = "scene"
	let externalScenes: string[] = []
	let hashModificationAllowed = true
	let customPaths: string[] = []

	export async function handleRequest(request: EntityMetadataRequest) {
		console.log(`Metadata editor for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "initialise":
				factory = request.data.factory
				blueprint = request.data.blueprint
				rootEntity = request.data.rootEntity
				subType = request.data.subType
				externalScenes = request.data.externalScenes
				break

			case "setFactory":
				factory = request.data.factory
				break

			case "setBlueprint":
				blueprint = request.data.blueprint
				break

			case "setHashModificationAllowed":
				hashModificationAllowed = request.data.hashModificationAllowed
				break

			case "updateCustomPaths":
				customPaths = request.data.customPaths
				break

			default:
				request satisfies never
				break
		}
	}

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				editor: editorID,
				data: {
					type: "entity",
					data: {
						type: "metadata",
						data: {
							type: "initialise"
						}
					}
				}
			}
		})
	})

	async function factoryHashInput(evt: any) {
		const _event = evt as { detail: string }

		await event({
			type: "editor",
			data: {
				editor: editorID,
				data: {
					type: "entity",
					data: {
						type: "metadata",
						data: {
							type: "setFactory",
							data: {
								factory: _event.detail
							}
						}
					}
				}
			}
		})
	}

	async function blueprintHashInput(evt: any) {
		const _event = evt as { detail: string }

		await event({
			type: "editor",
			data: {
				editor: editorID,
				data: {
					type: "entity",
					data: {
						type: "metadata",
						data: {
							type: "setBlueprint",
							data: {
								blueprint: _event.detail
							}
						}
					}
				}
			}
		})
	}

	async function rootEntityInput(evt: any) {
		const _event = evt as { detail: string }

		await event({
			type: "editor",
			data: {
				editor: editorID,
				data: {
					type: "entity",
					data: {
						type: "metadata",
						data: {
							type: "setRootEntity",
							data: {
								rootEntity: _event.detail
							}
						}
					}
				}
			}
		})
	}
</script>

<div class="h-full w-full overflow-y-auto" use:help={{ title: "Metadata", description: "This view lets you see and edit the metadata of an entity." }}>
	<TextInput bind:value={factory} placeholder="A hash or path" labelText="Factory" on:change={factoryHashInput} disabled={!hashModificationAllowed} class="code-font" />

	<div class="my-4">
		<TextInput bind:value={blueprint} placeholder="A hash or path" labelText="Blueprint" on:change={blueprintHashInput} disabled={!hashModificationAllowed} class="code-font" />
	</div>

	<div class="grid grid-cols-2 gap-2">
		<TextInput value={rootEntity} placeholder="The root sub-entity of this entity" labelText="Root entity" on:change={rootEntityInput} class="code-font" />

		<Dropdown
			titleText="Entity type"
			selectedId={subType}
			items={[
				{ id: "template", text: "Template" },
				{ id: "brick", text: "Brick" },
				{ id: "scene", text: "Scene" }
			]}
			on:select={async ({ detail }) => {
				await event({
					type: "editor",
					data: {
						editor: editorID,
						data: {
							type: "entity",
							data: {
								type: "metadata",
								data: {
									type: "setSubType",
									data: {
										subType: detail.selectedId
									}
								}
							}
						}
					}
				})
			}}
		/>
	</div>

	<h4 class="mt-4 mb-2">External scenes</h4>
	<ListEditor
		data={externalScenes}
		on:updated={async ({ detail }) => {
			await event({
				type: "editor",
				data: {
					editor: editorID,
					data: {
						type: "entity",
						data: {
							type: "metadata",
							data: {
								type: "setExternalScenes",
								data: {
									externalScenes: detail
								}
							}
						}
					}
				}
			})
		}}
	/>
</div>
