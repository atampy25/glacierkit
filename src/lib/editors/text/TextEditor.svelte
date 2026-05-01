<script lang="ts">
	import type { TextEditorRequest, TextFileType } from "$lib/bindings"
	import { event } from "$lib/utils"
	import Monaco from "./Monaco.svelte"

	export let id: string

	let monacoEditor: Monaco

	let fileType: TextFileType = "PlainText"

	export async function handleRequest(request: TextEditorRequest) {
		console.log(`Text editor ${id} handling request`, request)

		switch (request.type) {
			case "replaceContent":
				monacoEditor.setContent(request.data.content)
				break

			case "setFileType":
				fileType = request.data.fileType
				monacoEditor.setFileType(request.data.fileType)
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
				editor: id,
				data: {
					type: "text",
					data: {
						type: "updateContent",
						data: {
							content
						}
					}
				}
			}
		})
	}

	async function onReady() {
		await event({
			type: "editor",
			data: {
				editor: id,
				data: {
					type: "text",
					data: {
						type: "initialise"
					}
				}
			}
		})
	}
</script>

<Monaco {id} on:contentChanged={({ detail }) => contentChanged(detail)} bind:this={monacoEditor} on:ready={onReady} />
