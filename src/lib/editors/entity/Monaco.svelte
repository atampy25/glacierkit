<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import baseSchema from "./schema.json"
	import type { EntityMonacoRequest } from "$lib/bindings-types"
	import { cloneDeep, debounce, merge } from "lodash"
	import propertyTypeSchemas from "./property-type-schemas.json"
	import enums from "./enums.json"
	import { event } from "$lib/utils"

	let el: HTMLDivElement = null!
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let editorID: string

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

		editor = monaco.editor.create(el, {
			model: monaco.editor.createModel("Select an entity in the tree to edit it here.", "json", monaco.Uri.parse(`monaco-model://qn-subentity-${editorID}`)),
			roundedSelection: false,
			theme: "theme",
			minimap: {
				enabled: true
			},
			automaticLayout: true
		})

		destroyFunc.run = () => {
			editor.dispose()
		}

		monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
			...monaco.languages.json.jsonDefaults.diagnosticsOptions,
			schemas: [
				...monaco.languages.json.jsonDefaults.diagnosticsOptions.schemas!.filter((a) => a.uri !== "monaco-schema://qn-subentity"),
				{
					uri: "monaco-schema://qn-subentity",
					fileMatch: ["*subentity*"],
					schema: merge(cloneDeep(baseSchema), {
						$ref: "#/definitions/SubEntity",
						definitions: {
							SubEntity: {
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
													$ref: "#/definitions/Property"
												}
											]
										}
									},
									platformSpecificProperties: {
										additionalProperties: {
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
														$ref: "#/definitions/Property"
													}
												]
											}
										}
									}
								}
							}
						}
					})
				}
			]
		})

		editor.onDidChangeModelContent(
			debounce(async () => {
				await event({
					type: "editor",
					data: {
						type: "entity",
						data: {
							type: "monaco",
							data: {
								type: "updateContent",
								data: {
									id: editorID,
									content: editor.getValue({ preserveBOM: true, lineEnding: "\n" })
								}
							}
						}
					}
				})
			}, 1000)
		)
	})

	onDestroy(() => {
		if (editor) {
			editor.getModel()?.dispose()
		}
	})

	export async function handleRequest(request: EntityMonacoRequest) {
		console.log(`Tree for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "replaceContent":
				editor.setValue(request.data.content)
				break

			case "updateIntellisense":
				break

			default:
				request satisfies never
				break
		}
	}
</script>

<div bind:this={el} class="h-full w-full" />
