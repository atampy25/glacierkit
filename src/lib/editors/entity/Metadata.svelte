<script lang="ts">
	import type { EntityMetadataRequest, SubType } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { TextInput, Dropdown } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import ListEditor from "$lib/components/ListEditor.svelte"
	import md5 from "md5"

	export let editorID: string

	let factoryHash = ""
	let blueprintHash = ""
	let rootEntity = ""
	let subType: SubType = "scene"
	let externalScenes: string[] = []
	let hashModificationAllowed = true
	let customPaths: string[] = []

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

			case "setFactoryHash":
				factoryHash = request.data.factory_hash
				break

			case "setBlueprintHash":
				blueprintHash = request.data.blueprint_hash
				break

			case "setHashModificationAllowed":
				hashModificationAllowed = request.data.hash_modification_allowed
				break

			case "updateCustomPaths":
				customPaths = request.data.custom_paths
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
	<TextInput
		bind:value={factoryHash}
		placeholder="You can use the Text Tools panel to generate this"
		labelText="Factory hash"
		helperText={customPaths.find((a) => ("00" + md5(a.toLowerCase()).slice(2, 16)).toUpperCase() === factoryHash)}
		on:change={factoryHashInput}
		disabled={!hashModificationAllowed}
		class="code-font"
	/>

	<div class="my-4">
		<TextInput
			bind:value={blueprintHash}
			placeholder="You can use the Text Tools panel to generate this"
			labelText="Blueprint hash"
			helperText={customPaths.find((a) => ("00" + md5(a.toLowerCase()).slice(2, 16)).toUpperCase() === blueprintHash)}
			on:change={blueprintHashInput}
			disabled={!hashModificationAllowed}
			class="code-font"
		/>
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
