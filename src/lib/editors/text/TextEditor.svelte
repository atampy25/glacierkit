<script lang="ts">
	import type { TextEditorRequest, TextFileType } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { onMount } from "svelte"
	import Monaco from "./Monaco.svelte"

	export let id: string

	let monacoEditor: Monaco

	let fileType: TextFileType["type"] = "PlainText"

	export async function handleRequest(request: TextEditorRequest) {
		console.log(`Text editor ${id} handling request`, request)

		switch (request.type) {
			case "replaceContent":
				monacoEditor.setContent(request.data.content)
				break

			case "setFileType":
				fileType = request.data.file_type.type
				monacoEditor.setFileType(request.data.file_type)
				break

			default:
				request satisfies never
				break
		}
	}

	async function contentChanged(content: string) {
		if (fileType === "Json" || fileType === "ManifestJson") {
			try {
				JSON.parse(content)
			} catch {
				return
			}
		}

		await event({
			type: "editor",
			data: {
				type: "text",
				data: {
					type: "updateContent",
					data: {
						id,
						content
					}
				}
			}
		})
	}

	async function onReady() {
		await event({
			type: "editor",
			data: {
				type: "text",
				data: {
					type: "initialise",
					data: { id }
				}
			}
		})
	}
</script>

<div class="h-full mr-2 mb-2">
	<Monaco {id} on:contentChanged={({ detail }) => contentChanged(detail)} bind:this={monacoEditor} on:ready={onReady} />
</div>
