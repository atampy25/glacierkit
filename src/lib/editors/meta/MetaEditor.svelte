<script lang="ts">
	import type { TextEditorRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"

	export let id: string

	export async function handleRequest(request: TextEditorRequest) {
		console.log(`Meta editor ${id} handling request`, request)

		switch (request.type) {
			case "replaceContent":
				break

			default:
				request satisfies never
				break
		}
	}

	// Data offset
	// Compressed size and is-scrambled flag
	// Data size (final)
	// System memory requirement
	// Video memory requirement
	// Resource ID
	// Resource type
	// References chunk size
	// States chunk size
	// Dependencies
</script>
