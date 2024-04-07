<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy } from "svelte"
	import type { TextFileType } from "$lib/bindings-types"
	import { debounce } from "lodash"

	let el: HTMLDivElement = null!
	let editor: { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor } | { type: "code"; monaco: monaco.editor.IStandaloneCodeEditor } = null!

	export let id: string

	const dispatch = createEventDispatcher()

	let destroyFunc = { run: () => {} }

	onDestroy(() => {
		destroyFunc.run()
	})

	export function setContents(original: string, modified: string) {
		destroyFunc.run()

		setTimeout(() => {
			if (!original) {
				editor = {
					type: "code",
					monaco: monaco.editor.create(el, {
						model: monaco.editor.createModel(modified, "json", monaco.Uri.parse(`monaco-model://nondiff-${Math.random().toString(16)}`)),
						roundedSelection: false,
						theme: "theme",
						minimap: {
							enabled: true
						},
						automaticLayout: true,
						fontFamily: "Fira Code",
						fontLigatures: true,
						colorDecorators: true
					})
				}

				editor.monaco.onDidChangeModelContent(() => {
					dispatch("contentChanged", (editor as { type: "code"; monaco: monaco.editor.IStandaloneCodeEditor }).monaco.getValue({ preserveBOM: true, lineEnding: "\n" }))
				})

				destroyFunc.run = () => {
					;(editor as { type: "code"; monaco: monaco.editor.IStandaloneCodeEditor }).monaco.getModel()?.dispose()
					;(editor as { type: "code"; monaco: monaco.editor.IStandaloneCodeEditor }).monaco.dispose()
				}
			} else {
				editor = {
					type: "diff",
					monaco: monaco.editor.createDiffEditor(el, {
						roundedSelection: false,
						theme: "theme",
						minimap: {
							enabled: true
						},
						automaticLayout: true,
						fontFamily: "Fira Code",
						fontLigatures: true,
						colorDecorators: true
					})
				}

				editor.monaco.onDidUpdateDiff(() => {
					dispatch(
						"contentChanged",
						(editor as { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor }).monaco.getModel()!.modified.getValue(monaco.editor.EndOfLinePreference.LF, true)
					)
				})

				destroyFunc.run = () => {
					;(editor as { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor }).monaco.getModel()?.original.dispose()
					;(editor as { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor }).monaco.getModel()?.modified.dispose()
					;(editor as { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor }).monaco.dispose()
				}
				;(editor as { type: "diff"; monaco: monaco.editor.IStandaloneDiffEditor }).monaco.setModel({
					original: monaco.editor.createModel(original, "json", monaco.Uri.parse(`monaco-model://orig-${Math.random().toString(16)}`)),
					modified: monaco.editor.createModel(modified, "json", monaco.Uri.parse(`monaco-model://modified-${Math.random().toString(16)}`))
				})
			}
		}, 0)
	}
</script>

<div bind:this={el} class="h-full w-full" />
