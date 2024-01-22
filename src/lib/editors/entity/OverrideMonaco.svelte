<script lang="ts">
	import * as monaco from "monaco-editor"
	import { onDestroy, onMount } from "svelte"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import baseSchema from "./schema.json"
	import { cloneDeep, debounce, merge } from "lodash"
	import propertyTypeSchemas from "./property-type-schemas.json"
	import enums from "./enums.json"
	import { event } from "$lib/utils"

	let el: HTMLDivElement = null!
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let editorID: string

	export let mode: "propertyOverrides" | "overrideDeletes" | "pinConnectionOverrides" | "pinConnectionOverrideDeletes"

	let destroyFunc = { run: () => {} }

	let decorations: monaco.editor.IEditorDecorationsCollection
	let decorationsToCheck: [string, string][] = []

	const baseOverrideSchema = merge(cloneDeep(baseSchema), {
		definitions: {
			PropertyOverride: {
				properties: {
					properties: {
						additionalProperties: {
							anyOf: [
								...Object.entries(propertyTypeSchemas).map(([propType, valSchema]) => {
									return merge(cloneDeep(baseSchema.definitions.Property), {
										properties: {
											type: {
												const: propType
											},
											value: valSchema
										},
										default: {
											type: propType,
											value: valSchema.default
										}
									})
								}),
								...Object.entries(propertyTypeSchemas).map(([propType, valSchema]) => {
									return merge(cloneDeep(baseSchema.definitions.Property), {
										properties: {
											type: {
												const: `TArray<${propType}>`
											},
											value: { type: "array", items: valSchema }
										},
										default: {
											type: `TArray<${propType}>`,
											value: [valSchema.default]
										}
									})
								}),
								...Object.entries(enums).map(([propType, possibleValues]) => {
									return merge(cloneDeep(baseSchema.definitions.Property), {
										properties: {
											type: {
												const: propType
											},
											value: {
												enum: possibleValues
											}
										},
										default: {
											type: propType,
											value: possibleValues[0]
										}
									})
								}),
								...Object.entries(enums).map(([propType, possibleValues]) => {
									return merge(cloneDeep(baseSchema.definitions.Property), {
										properties: {
											type: {
												const: `TArray<${propType}>`
											},
											value: {
												type: "array",
												items: {
													enum: possibleValues
												}
											}
										},
										default: {
											type: `TArray<${propType}>`,
											value: [possibleValues[0]]
										}
									})
								}),
								{
									$ref: "#/definitions/OverriddenProperty"
								}
							]
						}
					}
				}
			}
		}
	})

	onDestroy(() => {
		destroyFunc.run()
	})

	onMount(async () => {
		editor = monaco.editor.create(el, {
			model: monaco.editor.createModel("Select an entity in the tree to edit it here.", "json", monaco.Uri.parse(`monaco-model://qn-override-${mode}-${editorID}`)),
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

		destroyFunc.run = () => {
			editor.dispose()
			editor.getModel()?.dispose()
		}

		monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
			...monaco.languages.json.jsonDefaults.diagnosticsOptions,
			schemas: [
				...monaco.languages.json.jsonDefaults.diagnosticsOptions.schemas!.filter((a) => !a.uri.startsWith("monaco-schema://qn-override-")),
				{
					uri: "monaco-schema://qn-override-propertyOverrides",
					fileMatch: ["*propertyoverrides*"],
					schema: merge(cloneDeep(baseOverrideSchema), {
						$ref: "#/definitions/Entity/properties/propertyOverrides"
					})
				},
				{
					uri: "monaco-schema://qn-override-overrideDeletes",
					fileMatch: ["*overridedeletes*"],
					schema: merge(cloneDeep(baseOverrideSchema), {
						$ref: "#/definitions/Entity/properties/overrideDeletes"
					})
				},
				{
					uri: "monaco-schema://qn-override-pinConnectionOverrides",
					fileMatch: ["*pinconnectionoverrides*"],
					schema: merge(cloneDeep(baseOverrideSchema), {
						$ref: "#/definitions/Entity/properties/pinConnectionOverrides"
					})
				},
				{
					uri: "monaco-schema://qn-override-pinConnectionOverrideDeletes",
					fileMatch: ["*pinconnectionoverridedeletes*"],
					schema: merge(cloneDeep(baseOverrideSchema), {
						$ref: "#/definitions/Entity/properties/pinConnectionOverrideDeletes"
					})
				}
			]
		})

		decorations = editor.createDecorationsCollection([])

		const debounced = debounce(async (content) => {
			await event({
				type: "editor",
				data: {
					type: "entity",
					data: {
						type: "overrides",
						data: {
							type: ("update" + mode[0].toUpperCase() + mode.slice(1)) as any,
							data: {
								editor_id: editorID,
								content
							}
						}
					}
				}
			})
		}, 1000)

		editor.onDidChangeModelContent(() => {
			debounced(editor.getValue({ preserveBOM: true, lineEnding: "\n" }))
			updateDecorations()
		})
	})

	function updateDecorations() {
		const newDecorations: monaco.editor.IModelDeltaDecoration[] = []

		for (const [no, line] of editor.getValue().split("\n").entries()) {
			for (const [check, deco] of decorationsToCheck) {
				if (line.includes(check)) {
					newDecorations.push({
						options: {
							isWholeLine: true,
							after: {
								content: " " + deco,
								cursorStops: monaco.editor.InjectedTextCursorStops.Left,
								inlineClassName: "monacoDecorationGray"
							}
						},
						range: new monaco.Range(no + 1, 0, no + 1, line.length + 1)
					})
				}
			}
		}

		decorations.set(newDecorations)
	}

	export function setDecorations(decorations: [string, string][]) {
		decorationsToCheck = decorations
		updateDecorations()
	}

	export function setContent(content: string) {
		editor.setValue(content)
	}
</script>

<div bind:this={el} class="overflow-hidden h-full" />
