<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import type { TextFileType } from "$lib/bindings-types"

	let el: HTMLDivElement = null!
	let Monaco: typeof monaco
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let id: string

	const dispatch = createEventDispatcher()

	let destroyFunc = { run: () => {} }

	onDestroy(() => {
		destroyFunc.run()
	})

	onMount(async () => {
		// @ts-ignore
		self.MonacoEnvironment = {
			getWorker: function (_moduleId: any, label: string) {
				if (label === "json") {
					return new jsonWorker()
				} else {
					return new editorWorker()
				}
			}
		}

		Monaco = await import("monaco-editor")

		Monaco.editor.defineTheme("theme", {
			base: "vs-dark",
			inherit: true,
			rules: [{ token: "keyword.json", foreground: "b5cea8" }],
			colors: {}
		})

		editor = Monaco.editor.create(el, {
			model: Monaco.editor.createModel("", "plaintext", Monaco.Uri.parse(`monaco-model://${id}`)),
			roundedSelection: false,
			theme: "theme",
			minimap: {
				enabled: true
			},
			automaticLayout: true
		})

		editor.onDidChangeModelContent(() => {
			dispatch("contentChanged", editor.getValue({ preserveBOM: true, lineEnding: "\n" }))
		})

		destroyFunc.run = () => {
			editor.dispose()
		}

		dispatch("ready")
	})

	onDestroy(() => {
		if (editor) {
			editor.getModel()?.dispose()
		}
	})

	export function setFileType(fileType: TextFileType) {
		const model = editor.getModel()

		if (model) {
			switch (fileType.type) {
				case "Json":
					Monaco.editor.setModelLanguage(model, "json")
					break

				case "ManifestJson":
					Monaco.editor.setModelLanguage(model, "json")

					try {
						;(async () => {
							Monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
								...Monaco.languages.json.jsonDefaults.diagnosticsOptions,
								schemas: [
									{
										uri: `monaco-schema://manifest`,
										fileMatch: [id],
										schema: await (await fetch("https://raw.githubusercontent.com/atampy25/simple-mod-framework/main/Mod%20Manager/src/lib/manifest-schema.json")).json()
									}
								]
							})
						})()
					} catch {}
					break

				case "PlainText":
					Monaco.editor.setModelLanguage(model, "plaintext")
					break

				case "Markdown":
					Monaco.editor.setModelLanguage(model, "markdown")
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

<div bind:this={el} class="h-[95%]" />
