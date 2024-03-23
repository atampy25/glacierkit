<script lang="ts">
	import type { EntityMetadataRequest, SubType } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { TextInput, Dropdown } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import ListEditor from "$lib/components/ListEditor.svelte"

	export let editorID: string

	let factoryHash = ""
	let blueprintHash = ""
	let rootEntity = ""
	let subType: SubType = "scene"
	let externalScenes: string[] = []
	let hashModificationAllowed = true

	export async function handleRequest(request: EntityMetadataRequest) {
		console.log(`Metadata editor for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "initialise":
				factoryHash = request.data.factory_hash
				blueprintHash = request.data.blueprint_hash
				rootEntity = request.data.root_entity
				subType = request.data.sub_type
				externalScenes = request.data.external_scenes
				break

			case "setHashModificationAllowed":
				hashModificationAllowed = request.data.hash_modification_allowed
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
				type: "entity",
				data: {
					type: "metadata",
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

	async function factoryHashInput(evt: any) {
		const _event = evt as { detail: string }

		await event({
			type: "editor",
			data: {
				type: "entity",
				data: {
					type: "metadata",
					data: {
						type: "setFactoryHash",
						data: {
							editor_id: editorID,
							factory_hash: _event.detail
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
				type: "entity",
				data: {
					type: "metadata",
					data: {
						type: "setBlueprintHash",
						data: {
							editor_id: editorID,
							blueprint_hash: _event.detail
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
				type: "entity",
				data: {
					type: "metadata",
					data: {
						type: "setRootEntity",
						data: {
							editor_id: editorID,
							root_entity: _event.detail
						}
					}
				}
			}
		})
	}
</script>

<div class="h-full w-full overflow-y-auto">
	<div class="grid grid-cols-2 lg:grid-cols-4 gap-2">
		<TextInput
			value={factoryHash}
			placeholder="You can use the Text Tools panel to generate this"
			labelText="Factory hash"
			on:change={factoryHashInput}
			disabled={!hashModificationAllowed}
			class="code-font"
		/>

		<TextInput
			value={blueprintHash}
			placeholder="You can use the Text Tools panel to generate this"
			labelText="Blueprint hash"
			on:change={blueprintHashInput}
			disabled={!hashModificationAllowed}
			class="code-font"
		/>

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
						type: "entity",
						data: {
							type: "metadata",
							data: {
								type: "setSubType",
								data: {
									editor_id: editorID,
									sub_type: detail.selectedId
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
					type: "entity",
					data: {
						type: "metadata",
						data: {
							type: "setExternalScenes",
							data: {
								editor_id: editorID,
								external_scenes: detail
							}
						}
					}
				}
			})
		}}
	/>
</div>
