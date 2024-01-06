<script lang="ts">
	export let id: string

	export async function handleRequest(request: EntityEditorRequest) {
		console.log(`Entity editor ${id} handling request`, request)

		switch (request.type) {
			case "replaceContent":
				break

			default:
				request satisfies never
				break
		}
	}
</script>

<div class="h-full flex items-center justify-center">
	<div class="text-center">
		<h1>This file can't be edited in Deeznuts.</h1>
		<p>You may be able to use VS Code or another tool to edit it.</p>
	</div>
</div>
