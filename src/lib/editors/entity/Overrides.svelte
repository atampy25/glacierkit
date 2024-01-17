<script lang="ts">
	import type { EntityOverridesRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { onMount } from "svelte"
	import OverrideMonaco from "./OverrideMonaco.svelte"

	export let editorID: string

	export async function handleRequest(request: EntityOverridesRequest) {
		console.log(`Overrides editor for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "initialise":
				propertyOverrideEditor.setContent(request.data.property_overrides)
				overrideDeleteEditor.setContent(request.data.override_deletes)
				pinConnectionOverrideEditor.setContent(request.data.pin_connection_overrides)
				pinConnectionOverrideDeleteEditor.setContent(request.data.pin_connection_override_deletes)
				break

			case "updateDecorations":
				propertyOverrideEditor.setDecorations(request.data.decorations)
				overrideDeleteEditor.setDecorations(request.data.decorations)
				pinConnectionOverrideEditor.setDecorations(request.data.decorations)
				pinConnectionOverrideDeleteEditor.setDecorations(request.data.decorations)
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
					type: "overrides",
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

	let activeMode: "propertyOverrides" | "overrideDeletes" | "pinConnectionOverrides" | "pinConnectionOverrideDeletes" = "propertyOverrides"

	let propertyOverrideEditor: OverrideMonaco
	let overrideDeleteEditor: OverrideMonaco
	let pinConnectionOverrideEditor: OverrideMonaco
	let pinConnectionOverrideDeleteEditor: OverrideMonaco
</script>

<div class="h-full w-full">
	<div class="h-10 bg-[#202020] flex flex-wrap w-fit mb-2">
		<div
			class="px-4 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
			class:border-b={activeMode === "propertyOverrides"}
			on:click={async () => {
				activeMode = "propertyOverrides"
			}}>Property overrides</div
		>
		<div
			class="px-4 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
			class:border-b={activeMode === "overrideDeletes"}
			on:click={async () => {
				activeMode = "overrideDeletes"
			}}>Override deletes</div
		>
		<div
			class="px-4 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
			class:border-b={activeMode === "pinConnectionOverrides"}
			on:click={async () => {
				activeMode = "pinConnectionOverrides"
			}}>Pin connection overrides</div
		>
		<div
			class="px-4 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
			class:border-b={activeMode === "pinConnectionOverrideDeletes"}
			on:click={async () => {
				activeMode = "pinConnectionOverrideDeletes"
			}}>Pin connection override deletes</div
		>
	</div>
	<div style="height: calc(100vh - 15rem)" class:hidden={activeMode !== "propertyOverrides"}>
		<OverrideMonaco {editorID} mode="propertyOverrides" bind:this={propertyOverrideEditor} />
	</div>
	<div style="height: calc(100vh - 15rem)" class:hidden={activeMode !== "overrideDeletes"}>
		<OverrideMonaco {editorID} mode="overrideDeletes" bind:this={overrideDeleteEditor} />
	</div>
	<div style="height: calc(100vh - 15rem)" class:hidden={activeMode !== "pinConnectionOverrides"}>
		<OverrideMonaco {editorID} mode="pinConnectionOverrides" bind:this={pinConnectionOverrideEditor} />
	</div>
	<div style="height: calc(100vh - 15rem)" class:hidden={activeMode !== "pinConnectionOverrideDeletes"}>
		<OverrideMonaco {editorID} mode="pinConnectionOverrideDeletes" bind:this={pinConnectionOverrideDeleteEditor} />
	</div>
</div>
