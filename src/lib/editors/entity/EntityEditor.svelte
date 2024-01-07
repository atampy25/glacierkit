<script lang="ts">
	import type { EntityEditorRequest } from "$lib/bindings-types"
	import Tree from "./Tree.svelte"

	export let id: string

	let tree: Tree

	export async function handleRequest(request: EntityEditorRequest) {
		console.log(`Entity editor ${id} handling request`, request)

		switch (request.type) {
			case "tree":
				tree.handleRequest(request.data)
				break

			default:
				request satisfies never
				break
		}
	}
</script>

<Tree editorID={id} bind:this={tree}></Tree>
