<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"

	let el: HTMLDivElement = null!
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let id: string
	export let content: string

	const dispatch = createEventDispatcher()

	let destroyFunc = { run: () => {} }

	onDestroy(() => {
		destroyFunc.run()
	})

	onMount(async () => {
		editor = monaco.editor.create(el, {
			model: monaco.editor.createModel("{}", "json", monaco.Uri.parse(`monaco-model://${id}`)),
			roundedSelection: false,
			theme: "theme",
			minimap: {
				enabled: true
			},
			automaticLayout: true,
			fontFamily: "Fira Code",
			fontLigatures: true,
			colorDecorators: true,
			readOnly: true,
			readOnlyMessage: {
				value: "Preview is read-only"
			}
		})

		destroyFunc.run = () => {
			editor.getModel()?.dispose()
			editor.dispose()
		}

		dispatch("ready")
	})

	$: editor?.setValue?.(content)
</script>

<div bind:this={el} class="h-full w-full" />
