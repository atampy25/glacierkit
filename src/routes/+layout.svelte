<script lang="ts">
	import "../app.css"
	import "../treeview.css"
	import "carbon-components-svelte/css/g90.css"
	import "@fortawesome/fontawesome-free/css/all.min.css"
	import "@fontsource/fira-code"
	import "$lib/crc32"

	import { appWindow } from "@tauri-apps/api/window"
	import { ComposedModal, ModalBody, ModalFooter, ModalHeader, SkipToContent, ToastNotification } from "carbon-components-svelte"
	import { listen } from "@tauri-apps/api/event"
	import { beforeUpdate, onDestroy } from "svelte"
	import { flip } from "svelte/animate"
	import { fade, fly } from "svelte/transition"
	import type { Property, Request } from "$lib/bindings-types"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import * as monaco from "monaco-editor"

	let tasks: [string, string][] = []
	let notifications: [string, { kind: "error" | "info" | "info-square" | "success" | "warning" | "warning-alt"; title: string; subtitle: string }][] = []

	let destroyFunc = { run: () => {} }
	onDestroy(() => {
		destroyFunc.run()
	})

	window.addEventListener("error", (evt) => {
		if (evt.error) {
			errorModalError = String(evt.error)
			errorModalOpen = true
			tasks = [...tasks.filter((a) => a[0] !== "error"), ["error", "App unstable, please backup current files on disk, save work and restart"]]
		}
	})

	let hasListened = false

	beforeUpdate(async () => {
		if (!hasListened) {
			hasListened = true

			const unlistenStartTask = await listen("start-task", ({ payload: task }: { payload: [string, string] }) => {
				tasks = [...tasks, task]
			})

			const unlistenFinishTask = await listen("finish-task", ({ payload: task }: { payload: string }) => {
				tasks = tasks.filter((a) => a[0] !== task)
			})

			const unlistedNotification = await listen("send-notification", ({ payload: notification }: { payload: (typeof notifications)[number] }) => {
				notifications = [...notifications, notification]
				setTimeout(() => {
					notifications = notifications.filter((a) => a[0] !== notification[0])
				}, 6000)
			})

			const unlistenRequest = await listen("request", ({ payload: request }: { payload: Request }) => {
				if (request.type === "global" && request.data.type === "setWindowTitle") {
					console.log("Layout handling request", request)

					appWindow.setTitle(`Deeznuts - ${request.data.data}`)
					windowTitle = request.data.data
				}

				if (request.type === "global" && request.data.type === "errorReport") {
					console.log("Layout handling request", request)

					errorModalError = request.data.data.error
					errorModalOpen = true
					tasks = [...tasks.filter((a) => a[0] !== "error"), ["error", "App unstable, please backup current files on disk, save work and restart"]]
				}
			})

			destroyFunc.run = () => {
				unlistenStartTask()
				unlistenFinishTask()
				unlistedNotification()
				unlistenRequest()
			}

			self.MonacoEnvironment = {
				getWorker: function (_moduleId: any, label: string) {
					if (label === "json") {
						return new jsonWorker()
					} else {
						return new editorWorker()
					}
				}
			}

			monaco.editor.defineTheme("theme", {
				base: "vs-dark",
				inherit: true,
				rules: [{ token: "keyword.json", foreground: "b5cea8" }],
				colors: {}
			})

			monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
				validate: true,
				enableSchemaRequest: true,
				schemas: [
					{
						uri: "monaco-schema://manifest",
						fileMatch: ["*manifest*"],
						schema: await (await fetch("https://raw.githubusercontent.com/atampy25/simple-mod-framework/main/Mod%20Manager/src/lib/manifest-schema.json")).json()
					},
					{
						uri: "monaco-schema://qn-subentity",
						fileMatch: ["*subentity*"],
						schema: {}
					}
				]
			})

			monaco.languages.registerColorProvider("json", {
				provideColorPresentations: (model, colorInfo) => {
					const color = colorInfo.color

					const r = Math.round(color.red * 255)
						.toString(16)
						.padStart(2, "0")

					const g = Math.round(color.green * 255)
						.toString(16)
						.padStart(2, "0")

					const b = Math.round(color.blue * 255)
						.toString(16)
						.padStart(2, "0")

					const a = Math.round(color.alpha * 255)
						.toString(16)
						.padStart(2, "0")

					// startLineNumber is 1-indexed so this gets the line just before the line with the colour value
					const includeAlpha = model.getLinesContent()[colorInfo.range.startLineNumber - 2].includes("SColorRGBA")

					return [
						{
							label: `#${r}${g}${b}${includeAlpha ? a : ""}`
						}
					]
				},

				provideDocumentColors: (model) => {
					try {
						const data = JSON.parse(model.getValue())

						const colours: monaco.languages.IColorInformation[] = []

						if (data.properties) {
							for (const propertyData of Object.values(data.properties) as Property[]) {
								if (propertyData.type === "SColorRGB" && typeof propertyData.value === "string" && propertyData.value.length === 7) {
									const r = parseInt(propertyData.value.slice(1).slice(0, 2), 16)
									const g = parseInt(propertyData.value.slice(1).slice(2, 4), 16)
									const b = parseInt(propertyData.value.slice(1).slice(4, 6), 16)

									for (const [lineNo, line] of model.getLinesContent().entries()) {
										const char = line.indexOf(propertyData.value)

										if (char !== -1) {
											colours.push({
												color: {
													red: r / 255,
													green: g / 255,
													blue: b / 255,
													alpha: 1
												},
												range: {
													startLineNumber: lineNo + 1,
													endLineNumber: lineNo + 1,
													startColumn: char + 1,
													endColumn: char + 1 + 7
												}
											})
										}
									}
								}

								if (propertyData.type === "SColorRGBA" && typeof propertyData.value === "string" && propertyData.value.length === 9) {
									const r = parseInt(propertyData.value.slice(1).slice(0, 2), 16)
									const g = parseInt(propertyData.value.slice(1).slice(2, 4), 16)
									const b = parseInt(propertyData.value.slice(1).slice(4, 6), 16)
									const a = parseInt(propertyData.value.slice(1).slice(6, 8), 16)

									for (const [lineNo, line] of model.getLinesContent().entries()) {
										const char = line.indexOf(propertyData.value)

										if (char !== -1) {
											colours.push({
												color: {
													red: r / 255,
													green: g / 255,
													blue: b / 255,
													alpha: a / 255
												},
												range: {
													startLineNumber: lineNo + 1,
													endLineNumber: lineNo + 1,
													startColumn: char + 1,
													endColumn: char + 1 + 9
												}
											})
										}
									}
								}
							}
						}

						if (data.platformSpecificProperties) {
							for (const platformData of Object.values(data.platformSpecificProperties) as Record<string, Property>[]) {
								for (const propertyData of Object.values(platformData)) {
									if (propertyData.type === "SColorRGB" && typeof propertyData.value === "string" && propertyData.value.length === 7) {
										const r = parseInt(propertyData.value.slice(1).slice(0, 2), 16)
										const g = parseInt(propertyData.value.slice(1).slice(2, 4), 16)
										const b = parseInt(propertyData.value.slice(1).slice(4, 6), 16)

										for (const [lineNo, line] of model.getLinesContent().entries()) {
											const char = line.indexOf(propertyData.value)

											if (char !== -1) {
												colours.push({
													color: {
														red: r / 255,
														green: g / 255,
														blue: b / 255,
														alpha: 1
													},
													range: {
														startLineNumber: lineNo + 1,
														endLineNumber: lineNo + 1,
														startColumn: char + 1,
														endColumn: char + 1 + 7
													}
												})
											}
										}
									}

									if (propertyData.type === "SColorRGBA" && typeof propertyData.value === "string" && propertyData.value.length === 9) {
										const r = parseInt(propertyData.value.slice(1).slice(0, 2), 16)
										const g = parseInt(propertyData.value.slice(1).slice(2, 4), 16)
										const b = parseInt(propertyData.value.slice(1).slice(4, 6), 16)
										const a = parseInt(propertyData.value.slice(1).slice(6, 8), 16)

										for (const [lineNo, line] of model.getLinesContent().entries()) {
											const char = line.indexOf(propertyData.value)

											if (char !== -1) {
												colours.push({
													color: {
														red: r / 255,
														green: g / 255,
														blue: b / 255,
														alpha: a / 255
													},
													range: {
														startLineNumber: lineNo + 1,
														endLineNumber: lineNo + 1,
														startColumn: char + 1,
														endColumn: char + 1 + 9
													}
												})
											}
										}
									}
								}
							}
						}

						const uniqueColours: monaco.languages.IColorInformation[] = []

						for (const colour of colours) {
							if (
								!uniqueColours.some(
									(a) => a.range.startColumn === colour.range.startColumn && a.range.endColumn === colour.range.endColumn && a.range.startLineNumber === colour.range.startLineNumber
								)
							) {
								uniqueColours.push(colour)
							}
						}

						return uniqueColours
					} catch {
						return []
					}
				}
			})
		}
	})

	let windowTitle = ""

	let errorModalOpen = false
	let errorModalError = ""
</script>

<ComposedModal
	open={errorModalOpen}
	on:submit={() => {
		errorModalOpen = false
	}}
>
	<ModalHeader title="Error" />
	<ModalBody>
		An error has occurred. Make a backup of your mod folder, then save any work inside this app and close the app to prevent further instability.
		<pre class="mt-2 p-4 bg-neutral-800 overflow-x-auto"><code>{errorModalError}</code></pre>
	</ModalBody>
	<ModalFooter danger primaryButtonText="Continue" />
</ComposedModal>

<header data-tauri-drag-region class:bx--header={true}>
	<SkipToContent />

	<!-- svelte-ignore a11y-missing-attribute -->
	<a data-tauri-drag-region class:bx--header__name={true}>Deeznuts</a>

	<div data-tauri-drag-region class="pointer-events-none cursor-none w-full text-center text-neutral-400">{windowTitle}</div>

	<div data-tauri-drag-region class="flex flex-row items-center justify-end text-white">
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.minimize}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M18 12H6" />
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.toggleMaximize}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M16.5 8.25V6a2.25 2.25 0 00-2.25-2.25H6A2.25 2.25 0 003.75 6v8.25A2.25 2.25 0 006 16.5h2.25m8.25-8.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-7.5A2.25 2.25 0 018.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 00-2.25 2.25v6"
				/>
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-red-600 active:bg-red-700" on:click={appWindow.close}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
			</svg>
		</div>
	</div>
</header>

<div class="w-full h-mid">
	<slot />
</div>

<div class="h-6 flex items-center gap-4 px-3 bg-neutral-600">
	{#if tasks.length}
		{#each tasks as [id, task] (id)}
			<span transition:fade={{ duration: 100 }} animate:flip={{ duration: 250 }}>{task}</span>
		{/each}
	{:else}
		<span>No tasks running</span>
	{/if}
</div>

<div class="absolute h-screen top-0 right-2" style="z-index: 9999">
	<div class="h-screen flex flex-col-reverse content-end pb-4">
		{#each notifications as [id, { kind, title, subtitle }] (id)}
			<div in:fly={{ x: 100 }} out:fade animate:flip>
				<ToastNotification hideCloseButton {kind} {title} {subtitle} />
			</div>
		{/each}
	</div>
</div>

<style>
	:global(.bx--header) {
		position: initial;
		display: flex;
		height: 3rem;
		align-items: center;
		border-bottom: 1px solid #393939;
		background-color: #161616;
	}

	.h-mid {
		height: calc(100vh - 3rem - 1.5rem);
	}

	:global(.bx--tooltip__trigger.bx--tooltip--right::after, .bx--tooltip__trigger .bx--assistive-text, .bx--tooltip__trigger + .bx--assistive-text) {
		background-color: #505050 !important;
		color: #f4f4f4 !important;
	}

	:global(.bx--tooltip__trigger.bx--tooltip--right::before) {
		border-color: rgba(0, 0, 0, 0) #505050 rgba(0, 0, 0, 0) rgba(0, 0, 0, 0) !important;
	}

	:global(.bx--tooltip__trigger.bx--tooltip--bottom::before, .bx--tooltip__trigger.bx--btn--icon-only--bottom.bx--tooltip--align-center::before) {
		border-color: rgba(0, 0, 0, 0) rgba(0, 0, 0, 0) #505050 rgba(0, 0, 0, 0) !important;
	}

	:global(.splitpanes__splitter) {
		position: relative;
		left: -4px;
	}

	:global(.splitpanes--vertical > .splitpanes__splitter) {
		cursor: col-resize;
		width: 8px;
	}

	:global(.splitpanes--horizontal > .splitpanes__splitter) {
		cursor: row-resize;
		height: 8px;
	}

	:global(.splitpanes__splitter:hover) {
		background-color: white;
		opacity: 10%;
		transition: background-color 100ms linear;
	}

	:global(:root) {
		color-scheme: dark;
	}

	:global(.jstree-node input) {
		color: white;
		outline: none !important;
	}

	:global(.jstree-default .jstree-search) {
		font-style: normal;
		font-weight: normal;
		@apply text-emerald-200;
	}

	:global(.jstree-default .jstree-hovered) {
		background: #3a3a3a;
		border-radius: 2px;
		box-shadow: none;
	}

	:global(.jstree-default .jstree-clicked) {
		background: #525252;
		border-radius: 2px;
		box-shadow: none;
	}

	:global(code) {
		font-family: "Fira Code", "IBM Plex Mono", "Menlo", "DejaVu Sans Mono", "Bitstream Vera Sans Mono", Courier, monospace;
	}

	:global(.bx--snippet code) {
		font-family: "Fira Code", "IBM Plex Mono", "Menlo", "DejaVu Sans Mono", "Bitstream Vera Sans Mono", Courier, monospace !important;
	}

	:global(.bx--toast-notification__caption) {
		display: none !important;
	}

	:global(.code-font) {
		font-family: "Fira Code", "IBM Plex Mono", "Menlo", "DejaVu Sans Mono", "Bitstream Vera Sans Mono", Courier, monospace !important;
	}
</style>
