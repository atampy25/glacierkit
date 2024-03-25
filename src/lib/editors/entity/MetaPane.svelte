<script lang="ts">
	import type { EntityMetaPaneRequest, ReverseReference } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { ClickableTile, TextArea } from "carbon-components-svelte"
	import { debounce } from "lodash"
	import { trackEvent } from "@aptabase/tauri"

	export let editorID: string

	let reverseRefs: ReverseReference[] = []
	let notesEntityID: string | null = null
	let notes = ""
	let entityNames: Record<string, string> = {}

	export async function handleRequest(request: EntityMetaPaneRequest) {
		console.log(`Meta pane for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "setReverseRefs":
				reverseRefs = request.data.reverse_refs
				entityNames = request.data.entity_names
				break

			case "setNotes":
				notesEntityID = request.data.entity_id
				notes = request.data.notes
				break

			default:
				request satisfies never
				break
		}
	}

	async function setNotes(entityID: string | null, value: string) {
		if (entityID) {
			trackEvent("Set entity notes")

			await event({
				type: "editor",
				data: {
					type: "entity",
					data: {
						type: "metaPane",
						data: {
							type: "setNotes",
							data: {
								editor_id: editorID,
								entity_id: entityID,
								notes: value
							}
						}
					}
				}
			})
		}
	}

	const debouncedSetNotes = debounce(setNotes, 1000)

	function notesInputHandler(evt: any) {
		const _event = evt as { target: HTMLTextAreaElement }

		debouncedSetNotes(notesEntityID, _event.target.value)
	}
</script>

<div class="h-full w-full flex flex-col gap-1 overflow-y-auto">
	<h3>Reverse references</h3>
	<div class="flex flex-wrap gap-2">
		{#each reverseRefs as ref}
			<ClickableTile
				on:click={async () => {
					trackEvent("Jump to reference from meta pane")

					await event({
						type: "editor",
						data: {
							type: "entity",
							data: {
								type: "metaPane",
								data: {
									type: "jumpToReference",
									data: {
										editor_id: editorID,
										reference: ref.from
									}
								}
							}
						}
					})
				}}
			>
				{#if ref.data.type === "parent"}
					<h4 class="-mt-1">Parent</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "property"}
					<h4 class="-mt-1">
						Property
						<span style="font-size: 1rem;">{ref.data.data.property_name}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "platformSpecificProperty"}
					<h4 class="-mt-1">
						Platform-Specific Property
						<span style="font-size: 1rem;">{ref.data.data.platform}: {ref.data.data.property_name}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "event"}
					<h4 class="-mt-1">
						Event
						<span style="font-size: 1rem;">{ref.data.data.event}/{ref.data.data.trigger}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "inputCopy"}
					<h4 class="-mt-1">
						Input Copy
						<span style="font-size: 1rem;">{ref.data.data.trigger}/{ref.data.data.propagate}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "outputCopy"}
					<h4 class="-mt-1">
						Output Copy
						<span style="font-size: 1rem;">{ref.data.data.event}/{ref.data.data.propagate}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "propertyAlias"}
					<h4 class="-mt-1">
						Property Alias
						<span style="font-size: 1rem;">{ref.data.data.original_property}/{ref.data.data.aliased_name}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "exposedEntity"}
					<h4 class="-mt-1">
						Exposed Entity
						<span style="font-size: 1rem;">{ref.data.data.exposed_name}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "exposedInterface"}
					<h4 class="-mt-1">
						Exposed Interface
						<span style="font-size: 1rem;">{ref.data.data.interface}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{:else if ref.data.type === "subset"}
					<h4 class="-mt-1">
						Entity Subset
						<span style="font-size: 1rem;">{ref.data.data.subset}</span>
					</h4>
					{entityNames[ref.from]} (<code>{ref.from}</code>)
				{/if}
			</ClickableTile>
		{/each}
		{#if !reverseRefs.length}
			<p>There aren't any reverse references for this entity.</p>
		{/if}
	</div>
	<h3 class="mt-2">Notes</h3>
	<TextArea placeholder="Notes about this entity, purely for your own reference." on:input={notesInputHandler} bind:value={notes} />
</div>
