<script lang="ts">
	import type { EntityEditorRequest } from "$lib/bindings-types"
	import { Pane, Splitpanes } from "svelte-splitpanes"
	import Tree from "./Tree.svelte"
	import Monaco from "./Monaco.svelte"

	export let id: string

	let tree: Tree
	let monaco: Monaco

	export async function handleRequest(request: EntityEditorRequest) {
		console.log(`Entity editor ${id} handling request`, request)

		switch (request.type) {
			case "tree":
				tree.handleRequest(request.data)
				break

			case "monaco":
				monaco.handleRequest(request.data)
				break

			default:
				request satisfies never
				break
		}
	}

	const modes = ["Metadata", "Overrides", "Tree"] as const
	let activeMode: (typeof modes)[number] = "Tree"
</script>

<div class="w-full h-full flex flex-col gap-2">
	<div class="flex-shrink-0 flex flex-wrap gap-4">
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
	</div>
	<div class="flex-grow grid grid-cols-4 gap-4">
		<div class="h-full w-full">
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
					<div class="h-full w-full flex flex-col gap-1">
						<h3>Reverse references</h3>
					</div>
				</Pane>
			</Splitpanes>
		</div>
		<div class="col-span-3 h-full w-full flex flex-col">
			<h3>Editor</h3>
			<div class="flex-grow w-full">
				<Monaco editorID={id} bind:this={monaco} />
			</div>
		</div>
	</div>
</div>
