<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import type { TextFileType } from "$lib/bindings-types"
	import { debounce } from "lodash"

	let el: HTMLDivElement = null!
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let id: string

	const dispatch = createEventDispatcher()

	let destroyFunc = { run: () => {} }

	onDestroy(() => {
		destroyFunc.run()
	})

	onMount(async () => {
		editor = monaco.editor.create(el, {
			model: monaco.editor.createModel("", "plaintext", monaco.Uri.parse(`monaco-model://${id}`)),
			roundedSelection: false,
			theme: "theme",
			minimap: {
				enabled: true
			},
			automaticLayout: true
		})

		editor.onDidChangeModelContent(
			debounce(() => {
				dispatch("contentChanged", editor.getValue({ preserveBOM: true, lineEnding: "\n" }))
			}, 1000)
		)

		destroyFunc.run = () => {
			editor.getModel()?.dispose()
			editor.dispose()
		}

		dispatch("ready")
	})

	export function setFileType(fileType: TextFileType) {
		const model = editor.getModel()

		if (model) {
			switch (fileType.type) {
				case "Json":
					monaco.editor.setModelLanguage(model, "json")
					break

				case "ManifestJson":
					editor.setModel(monaco.editor.createModel(editor.getValue(), "json", monaco.Uri.parse(`monaco-model://manifest-${id}`)))
					break

				case "PlainText":
					monaco.editor.setModelLanguage(model, "plaintext")
					break

				case "Markdown":
					monaco.editor.setModelLanguage(model, "markdown")
					break

				default:
					fileType satisfies never
					break
			}
		}
	}

	export function setContent(content: string) {
		editor.setValue(content)
	}
</script>

<div bind:this={el} class="h-full w-full" />
