<script lang="ts">
	import * as monaco from "monaco-editor"
	import { createEventDispatcher, onDestroy, onMount } from "svelte"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import baseSchema from "./schema.json"
	import type { EditorValidity, EntityMonacoRequest, JsonValue } from "$lib/bindings-types"
	import { cloneDeep, debounce, merge } from "lodash"
	import propertyTypeSchemas from "./property-type-schemas.json"
	import enums from "./enums.json"
	import { event } from "$lib/utils"
	import { listen } from "@tauri-apps/api/event"
	import { Modal } from "carbon-components-svelte"
	import GraphRenderer from "./GraphRenderer.svelte"
	import { trackEvent } from "@aptabase/tauri"

	let el: HTMLDivElement = null!
	let editor: monaco.editor.IStandaloneCodeEditor = null!

	export let editorID: string

	let entityID: string | null = null
	let validity: EditorValidity = { type: "Valid" }

	let destroyFunc = { run: () => {} }

	let decorations: monaco.editor.IEditorDecorationsCollection

	let decorationsToCheck: [string, string][] = []
	let localRefEntityIDs: string[] = []

	let showCurvePreview = false
	let curveToPreview: [number, number, number, number, number, number, number, number][] | null = null

	let editorConnected = false

	const baseIntellisenseSchema = merge(cloneDeep(baseSchema), {
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
										...(Object.hasOwn(valSchema, "default")
											? {
													default: {
														type: propType,
														value: valSchema.default
													}
												}
											: {})
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
										...(Object.hasOwn(valSchema, "default")
											? {
													default: {
														type: `TArray<${propType}>`,
														value: [valSchema.default]
													}
												}
											: {})
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
											...(Object.hasOwn(valSchema, "default")
												? {
														default: {
															type: propType,
															value: valSchema.default
														}
													}
												: {})
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
											...(Object.hasOwn(valSchema, "default")
												? {
														default: {
															type: `TArray<${propType}>`,
															value: [valSchema.default]
														}
													}
												: {})
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

	onDestroy(() => {
		destroyFunc.run()
	})

	const debouncedUpdateFunction = { run: debounce(async (_: string) => {}, 250) }

	onMount(async () => {
		editor = monaco.editor.create(el, {
			model: monaco.editor.createModel("Select an entity in the tree to edit it here.", "json", monaco.Uri.parse(`monaco-model://qn-subentity-${editorID}`)),
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

		const showPreviewCurveCondition = editor.createContextKey<boolean>("showPreviewCurveCondition", false)
		const showFollowReferenceCondition = editor.createContextKey<boolean>("showFollowReferenceCondition", false)
		const showOpenFactoryCondition = editor.createContextKey<boolean>("showOpenFactoryCondition", false)
		const showSignalPinCondition = editor.createContextKey<boolean>("showSignalPinCondition", false)
		const showOpenResourceOverviewCondition = editor.createContextKey<boolean>("showOpenResourceOverviewCondition", false)

		editor.onDidChangeCursorPosition((e) => {
			let entData
			try {
				entData = JSON.parse(editor.getValue())
			} catch {
				return
			}

			let word: string | undefined | false
			try {
				word = editor.getModel()!.getWordAtPosition(e.position)?.word
			} catch {
				word = false
			}

			if (!word) {
				showPreviewCurveCondition.set(false)
			} else {
				showPreviewCurveCondition.set(entData.properties && entData.properties[word] && entData.properties[word].type === "ZCurve")
			}

			if (!word) {
				showFollowReferenceCondition.set(false)
			} else {
				showFollowReferenceCondition.set(localRefEntityIDs.includes(word))
			}

			showOpenFactoryCondition.set(editor.getModel()!.getLineContent(e.position.lineNumber).includes(`"factory":`))

			if (!word || !editorConnected) {
				showSignalPinCondition.set(false)
			} else {
				showSignalPinCondition.set([...Object.keys(entData.inputCopying || {}), ...Object.keys(entData.outputCopying || {}), ...Object.keys(entData.events || {})].includes(word))
			}

			showOpenResourceOverviewCondition.set(
				!editor.getModel()!.getLineContent(e.position.lineNumber).includes(`"factory":`) &&
					(editor.getModel()!.getLineContent(e.position.lineNumber).includes("assembly:/") ||
						editor.getModel()!.getLineContent(e.position.lineNumber).includes("modules:/") ||
						/"00[0-9A-F]{14}"/.test(editor.getModel()!.getLineContent(e.position.lineNumber)))
			)
		})

		editor.addAction({
			id: "preview-curve",
			label: "Visualise curve",
			contextMenuGroupId: "navigation",
			contextMenuOrder: 0,
			keybindings: [],
			precondition: "showPreviewCurveCondition",
			run: async (ed) => {
				trackEvent("Visualise curve")

				const propertyName = editor.getModel()!.getWordAtPosition(ed.getPosition()!)!.word

				curveToPreview = JSON.parse(editor.getValue()).properties[propertyName].value.data

				showCurvePreview = true
			}
		})

		editor.addAction({
			id: "signal-pin",
			label: "Signal pin in-game",
			contextMenuGroupId: "navigation",
			contextMenuOrder: 0,
			keybindings: [],
			precondition: "showSignalPinCondition",
			run: async (ed) => {
				try {
					const pin = editor.getModel()!.getWordAtPosition(ed.getPosition()!)!.word
					const output = !JSON.parse(editor.getValue()).inputCopying[pin]

					trackEvent("Signal pin in-game")

					await event({
						type: "editor",
						data: {
							type: "entity",
							data: {
								type: "monaco",
								data: {
									type: "signalPin",
									data: {
										editor_id: editorID,
										entity_id: entityID!,
										pin,
										output
									}
								}
							}
						}
					})
				} catch {}
			}
		})

		editor.addAction({
			id: "follow-reference",
			label: "Follow reference",
			contextMenuGroupId: "navigation",
			contextMenuOrder: 0,
			keybindings: [monaco.KeyCode.F12],
			precondition: "showFollowReferenceCondition",
			run: async (ed) => {
				trackEvent("Follow reference in Monaco editor")

				await event({
					type: "editor",
					data: {
						type: "entity",
						data: {
							type: "monaco",
							data: {
								type: "followReference",
								data: {
									editor_id: editorID,
									reference: editor.getModel()!.getWordAtPosition(ed.getPosition()!)!.word
								}
							}
						}
					}
				})
			}
		})

		editor.addAction({
			id: "open-factory",
			label: "Open factory in new tab",
			contextMenuGroupId: "navigation",
			contextMenuOrder: 0,
			keybindings: [monaco.KeyCode.F12],
			precondition: "showOpenFactoryCondition",
			run: async (ed) => {
				trackEvent("Open factory in new tab from Monaco editor")

				await event({
					type: "editor",
					data: {
						type: "entity",
						data: {
							type: "monaco",
							data: {
								type: "openFactory",
								data: {
									editor_id: editorID,
									factory: JSON.parse(editor.getValue()).factory
								}
							}
						}
					}
				})
			}
		})

		editor.addAction({
			id: "open-resource-overview",
			label: "Open resource overview in new tab",
			contextMenuGroupId: "navigation",
			contextMenuOrder: 0,
			keybindings: [monaco.KeyCode.F12],
			precondition: "showOpenResourceOverviewCondition",
			run: async (ed) => {
				trackEvent("Open resource overview in new tab from Monaco editor")

				await event({
					type: "editor",
					data: {
						type: "entity",
						data: {
							type: "monaco",
							data: {
								type: "openResourceOverview",
								data: {
									editor_id: editorID,
									resource: [
										...editor
											.getModel()!
											.getLineContent(ed.getPosition()!.lineNumber)
											.matchAll(/"(.*?)"/g)
									].at(-1)![1]
								}
							}
						}
					}
				})
			}
		})

		monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
			...monaco.languages.json.jsonDefaults.diagnosticsOptions,
			schemas: [
				...monaco.languages.json.jsonDefaults.diagnosticsOptions.schemas!.filter((a) => a.uri !== `monaco-schema://qn-subentity-${editorID}`),
				{
					uri: `monaco-schema://qn-subentity-${editorID}`,
					fileMatch: ["*subentity*"],
					schema: baseIntellisenseSchema
				}
			]
		})

		decorations = editor.createDecorationsCollection([])

		editor.onDidChangeModelContent((e) => {
			debouncedUpdateFunction.run(editor.getValue({ preserveBOM: true, lineEnding: "\n" }))
			updateDecorations()
		})
	})

	function updateIntellisense(data: { properties: [string, string, JsonValue, boolean][]; pins: [string[], string[]] }) {
		monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
			...monaco.languages.json.jsonDefaults.diagnosticsOptions,
			schemas: [
				...monaco.languages.json.jsonDefaults.diagnosticsOptions.schemas!.filter((a) => a.uri !== `monaco-schema://qn-subentity-${editorID}`),
				{
					uri: `monaco-schema://qn-subentity-${editorID}`,
					fileMatch: [`*subentity-${editorID}*`],
					schema: merge(cloneDeep(baseIntellisenseSchema), {
						definitions: {
							SubEntity: {
								properties: {
									properties: {
										properties: Object.fromEntries(
											data.properties.map(([name, type, defaultValue, postInit]) => {
												return [
													name,
													{
														type: "object",
														properties: {
															type: {
																type: "string",
																const: type
															},
															value: merge(
																cloneDeep(
																	(enums as Record<string, string[]>)[type]
																		? { enum: (enums as Record<string, string[]>)[type] }
																		: (propertyTypeSchemas as Record<string, any>)[type] || {}
																),
																{
																	default: defaultValue
																}
															),
															postInit: {
																type: "boolean",
																default: postInit
															}
														},
														required: ["type", "value"],
														default: {
															type,
															value: defaultValue,
															postInit: postInit || undefined
														}
													}
												]
											})
										)
									},
									events: {
										properties: Object.fromEntries(
											data.pins[1].map((a) => [
												a,
												{
													type: "object",
													additionalProperties: {
														type: "array",
														items: {
															$ref: "#/definitions/RefMaybeConstantValue"
														}
													}
												}
											])
										)
									},
									inputCopying: {
										properties: Object.fromEntries(
											data.pins[0].map((a) => [
												a,
												{
													type: "object",
													additionalProperties: {
														type: "array",
														items: {
															$ref: "#/definitions/RefMaybeConstantValue"
														}
													}
												}
											])
										)
									},
									outputCopying: {
										properties: Object.fromEntries(
											data.pins[1].map((a) => [
												a,
												{
													type: "object",
													additionalProperties: {
														type: "array",
														items: {
															$ref: "#/definitions/RefMaybeConstantValue"
														}
													}
												}
											])
										)
									}
								}
							}
						}
					})
				}
			]
		})
	}

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

	export async function handleRequest(request: EntityMonacoRequest) {
		console.log(`Monaco editor for editor ${editorID} handling request`, request)

		switch (request.type) {
			case "deselectIfSelected":
				if (request.data.entity_ids.includes(entityID || "")) {
					entityID = null
				}
				break

			case "replaceContent":
				entityID = request.data.entity_id
				debouncedUpdateFunction.run = debounce(async (content: string) => {
					await event({
						type: "editor",
						data: {
							type: "entity",
							data: {
								type: "monaco",
								data: {
									type: "updateContent",
									data: {
										editor_id: editorID,
										entity_id: request.data.entity_id,
										content
									}
								}
							}
						}
					})
				}, 250)
				editor.setValue(request.data.content)
				break

			case "replaceContentIfSameEntityID":
				if (entityID === request.data.entity_id) {
					editor.setValue(request.data.content)
				}
				break

			case "updateValidity":
				validity = request.data.validity
				break

			case "updateIntellisense":
				// Relies on the intellisense request getting processed after the content replacement request but that should be the case, since intellisense is fairly slow
				if (request.data.entity_id === entityID) {
					updateIntellisense({
						properties: request.data.properties,
						pins: request.data.pins
					})
				}
				break

			case "updateDecorationsAndMonacoInfo":
				if (request.data.entity_id === entityID) {
					decorationsToCheck = request.data.decorations
					localRefEntityIDs = request.data.local_ref_entity_ids
					updateDecorations()
				}
				break

			case "setEditorConnected":
				editorConnected = request.data.connected
				break

			default:
				request satisfies never
				break
		}
	}
</script>

<div class="flex flex-wrap gap-2 mb-1" class:hidden={entityID === null}>
	<code>{entityID}</code>
	{#if validity.type === "Valid"}
		<span class="text-green-200">Valid entity</span>
	{:else}
		<span class="text-red-200">{validity.data}</span>
	{/if}
</div>
<div bind:this={el} class="overflow-visible" style="height: calc(100vh - 11rem - 2rem - 0.5rem - 2.25rem)" class:hidden={entityID === null} />
{#if entityID === null}
	<p>Select an entity on the left to edit it here.</p>
{/if}

<Modal passiveModal bind:open={showCurvePreview} modalHeading="Curve preview">
	{#if curveToPreview}
		<GraphRenderer {curveToPreview} />
	{/if}
</Modal>

<style>
	:global(.monacoDecorationGray) {
		color: #858585 !important;
	}
</style>
