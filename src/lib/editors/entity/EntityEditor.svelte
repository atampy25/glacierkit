<script lang="ts">
	import type { EntityEditorRequest } from "$lib/bindings-types"
	import { Pane, Splitpanes } from "svelte-splitpanes"
	import Tree from "./Tree.svelte"
	import Monaco from "./Monaco.svelte"
	import MetaPane from "./MetaPane.svelte"
	import { Checkbox } from "carbon-components-svelte"
	import { event } from "$lib/utils"
	import Metadata from "./Metadata.svelte"
	import Overrides from "./Overrides.svelte"

	export let id: string

	let tree: Tree
	let monaco: Monaco
	let metaPane: MetaPane
	let metadata: Metadata
	let overrides: Overrides

	export async function handleRequest(request: EntityEditorRequest) {
		console.log(`Entity editor ${id} handling request`, request)

		switch (request.type) {
			case "tree":
				tree.handleRequest(request.data)
				break

			case "monaco":
				monaco.handleRequest(request.data)
				break

			case "metaPane":
				metaPane.handleRequest(request.data)
				break

			case "metadata":
				metadata.handleRequest(request.data)
				break

			case "overrides":
				overrides.handleRequest(request.data)
				break

			case "general":
				isPatchEditor = request.data.data.is_patch_editor
				break

			default:
				request satisfies never
				break
		}
	}

	const modes = ["Metadata", "Overrides", "Tree"] as const
	let activeMode: (typeof modes)[number] = "Tree"

	let showReverseParentRefs = false
	let showChangesFromOriginal = false

	let isPatchEditor = false

	async function showReverseParentRefsChanged(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		await event({
			type: "editor",
			data: {
				type: "entity",
				data: {
					type: "general",
					data: {
						type: "setShowReverseParentRefs",
						data: {
							editor_id: id,
							show_reverse_parent_refs: _event.target.checked
						}
					}
				}
			}
		})
	}

	async function showChangesFromOriginalChanged(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		await event({
			type: "editor",
			data: {
				type: "entity",
				data: {
					type: "general",
					data: {
						type: "setShowChangesFromOriginal",
						data: {
							editor_id: id,
							show_changes_from_original: _event.target.checked
						}
					}
				}
			}
		})
	}
</script>

<div class="w-full h-full">
	<div class="flex-shrink-0 flex flex-wrap gap-4 mb-2 items-center">
		<div class="h-10 bg-[#202020] flex flex-wrap w-fit">
			{#each modes as mode}
				<div
					class="px-4 flex gap-2 items-center justify-center cursor-pointer border-solid border-b-white"
					class:border-b={activeMode === mode}
					on:click={async () => {
						activeMode = mode
					}}>{mode}</div
				>
			{/each}
		</div>
		<div>
			<Checkbox checked={showReverseParentRefs} on:change={showReverseParentRefsChanged} labelText="Show reverse parent references" />
		</div>
		{#if isPatchEditor}
			<div>
				<Checkbox checked={showChangesFromOriginal} on:change={showChangesFromOriginalChanged} labelText="Highlight changes from original entity" />
			</div>
		{/if}
	</div>
	<div style="height: calc(100vh - 11rem)" class:hidden={activeMode !== "Metadata"}>
		<Metadata editorID={id} bind:this={metadata} />
	</div>
	<div style="height: calc(100vh - 11rem)" class:hidden={activeMode !== "Overrides"}>
		<Overrides editorID={id} bind:this={overrides} />
	</div>
	<div style="height: calc(100vh - 11rem)" class:hidden={activeMode !== "Tree"}>
		<Splitpanes theme="">
			<Pane size={25}>
				<div class="w-full h-full pb-4 pr-2">
					<Splitpanes horizontal theme="">
						<Pane size={80}>
							<div class="h-full w-full flex flex-col gap-1">
								<h3>Tree</h3>
								<!-- The `min-h-0 basis-0` here is EXTREMELY necessary as the tree will refuse to apply overflow-auto if it is removed, instead extending the box past its allowance! -->
								<div class="flex-grow flex flex-col gap-2 min-h-0 basis-0">
									<Tree editorID={id} bind:this={tree} />
								</div>
							</div>
						</Pane>
						<Pane>
							<MetaPane editorID={id} bind:this={metaPane} />
						</Pane>
					</Splitpanes>
				</div>
			</Pane>
			<Pane class="overflow-visible">
				<div class="h-full w-full flex flex-col gap-1">
					<h3>Editor</h3>
					<Monaco editorID={id} bind:this={monaco} />
				</div>
			</Pane>
		</Splitpanes>
	</div>
</div>

<style>
	:global(.splitpanes__pane.overflow-visible) {
		overflow: visible;
	}
</style>
