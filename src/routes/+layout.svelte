<script lang="ts">
	import "../app.css"
	import "../treeview.css"
	import "carbon-components-svelte/css/g90.css"
	import "@fortawesome/fontawesome-free/css/all.min.css"
	import "@fontsource/fira-code"
	import "$lib/crc32"

	import { appWindow } from "@tauri-apps/api/window"
	import { ComposedModal, HeaderNavItem, ModalBody, ModalFooter, ModalHeader, SkipToContent, ToastNotification } from "carbon-components-svelte"
	import { listen } from "@tauri-apps/api/event"
	import { beforeUpdate, onDestroy } from "svelte"
	import { flip } from "svelte/animate"
	import { fade, fly } from "svelte/transition"
	import type { Property, Request } from "$lib/bindings-types"
	import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker"
	import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker"
	import * as monaco from "monaco-editor"
	import { createPatch } from "rfc6902"
	import { writeTextFile } from "@tauri-apps/api/fs"
	import { attachConsole, info } from "tauri-plugin-log"
	import { help } from "$lib/helpray"
	import HelpRay from "$lib/components/HelpRay.svelte"
	import { trackEvent } from "@aptabase/tauri"
	import { checkUpdate, installUpdate, type UpdateManifest } from "@tauri-apps/api/updater"
	import { getVersion } from "@tauri-apps/api/app"
	import { relaunch } from "@tauri-apps/api/process"
	import { event } from "$lib/utils"

	let tasks: [string, string][] = []
	let notifications: [string, { kind: "error" | "info" | "info-square" | "success" | "warning" | "warning-alt"; title: string; subtitle: string }][] = []

	let destroyFunc = { run: () => {} }
	onDestroy(() => {
		destroyFunc.run()
	})

	window.addEventListener("error", (evt) => {
		if (evt.error) {
			void trackEvent("Frontend error", { error: String(evt.error), stack: evt.error.stack })

			errorModalError = `${String(evt.error)}, ${evt.error.stack}`
			errorModalOpen = true
			tasks = [...tasks.filter((a) => a[0] !== "error"), ["error", "App unstable, please backup current files on disk, save work and restart"]]
		}
	})

	let hasListened = false

	beforeUpdate(async () => {
		if (!hasListened) {
			hasListened = true

			const detachConsole = await attachConsole()

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

					appWindow.setTitle(`GlacierKit - ${request.data.data}`)
					windowTitle = request.data.data
				}

				if (request.type === "global" && request.data.type === "errorReport") {
					console.log("Layout handling request", request)

					void trackEvent("Error", { error: request.data.data.error })

					errorModalError = request.data.data.error
					errorModalOpen = true
					tasks = [...tasks.filter((a) => a[0] !== "error"), ["error", "App unstable, please backup current files on disk, save work and restart"]]
				}

				// Because rfc6902 is the only patch creation library which properly handles arrays
				if (request.type === "global" && request.data.type === "computeJSONPatchAndSave") {
					console.log("Layout handling request", request)

					const patch = createPatch(request.data.data.base, request.data.data.current)

					void writeTextFile(
						request.data.data.save_path,
						JSON.stringify({
							file: request.data.data.file_and_type[0],
							type: request.data.data.file_and_type[1],
							patch
						})
					)
				}

				if (request.type === "global" && request.data.type === "requestLastPanicUpload") {
					console.log("Layout handling request", request)

					lastPanicModalOpen = true
				}

				if (request.type === "global" && request.data.type === "logUploadRejected") {
					console.log("Layout handling request", request)

					logUploadRejectedModalOpen = true
				}
			})

			destroyFunc.run = () => {
				unlistenStartTask()
				unlistenFinishTask()
				unlistedNotification()
				unlistenRequest()
				detachConsole()
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

			let manifestSchema = {}

			try {
				manifestSchema = await (await fetch("https://raw.githubusercontent.com/atampy25/simple-mod-framework/main/Mod%20Manager/src/lib/manifest-schema.json")).json()
			} catch (e) {
				info(`Couldn't get manifest schema: ${String(e)}, ${e.stack}`)
			}

			monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
				validate: true,
				enableSchemaRequest: true,
				schemas: [
					{
						uri: "monaco-schema://manifest",
						fileMatch: ["*manifest*"],
						schema: manifestSchema
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

			appWindow.show()

			try {
				const { shouldUpdate, manifest } = await checkUpdate()

				if (shouldUpdate) {
					updateManifest = manifest!

					const currentVersion = await getVersion()

					const commits = await (
						await fetch("https://api.github.com/repos/atampy25/glacierkit/commits", {
							headers: {
								Accept: "application/vnd.github.v3+json"
							}
						})
					).json()

					commits.reverse()

					const prevVersionCommit = await (
						await fetch(`https://api.github.com/repos/atampy25/glacierkit/commits/${currentVersion}`, {
							headers: {
								Accept: "application/vnd.github.v3+json"
							}
						})
					).json()

					// Exclude last version commit and its post-update commit
					commitsSinceLastVersion = commits
						.slice(commits.findIndex((a: { sha: string }) => a.sha === prevVersionCommit.sha) + 2)
						.map((a: { commit: { message: string } }) => a.commit.message)
						.filter((a: string) => a !== "Post-update")

					updateModalOpen = true
				}
			} catch (e) {
				info(`Ignoring error in update checking: ${String(e)}, ${e.stack}`)
			}
		}
	})

	let windowTitle = ""

	let errorModalOpen = false
	let errorModalError = ""

	let helpRayActive = false

	let updateModalOpen = false
	let updateManifest: UpdateManifest = { version: "", date: "", body: "" }
	let commitsSinceLastVersion: string[] = []

	let lastPanicModalOpen = false

	let logUploadRejectedModalOpen = false
</script>

<ComposedModal
	open={errorModalOpen}
	on:click:button--primary={async () => {
		errorModalOpen = false

		await event({
			type: "global",
			data: {
				type: "uploadLogAndReport",
				data: errorModalError
			}
		})
	}}
>
	<ModalHeader title="Error" />
	<ModalBody>
		An error has occurred. Make a backup of your mod folder, then save any work inside this app and close the app to prevent further instability. You can send your log file to Atampy26
		automatically to help fix this issue. If you choose not to, you can find it in <code>%appdata%\app.glacierkit\logs</code>.
		<pre class="mt-2 p-4 bg-neutral-800 overflow-x-auto"><code>{errorModalError}</code></pre>
	</ModalBody>
	<ModalFooter
		primaryButtonText="Upload log and continue"
		secondaryButtonText="Continue without uploading log"
		on:click:button--secondary={() => {
			errorModalOpen = false
		}}
	/>
</ComposedModal>

<ComposedModal
	open={lastPanicModalOpen}
	on:click:button--primary={async () => {
		lastPanicModalOpen = false

		await event({
			type: "global",
			data: {
				type: "uploadLastPanic"
			}
		})
	}}
>
	<ModalHeader title="Crash report" />
	<ModalBody>
		It seems GlacierKit crashed the last time it was used. You can send a crash report, including your log and the error message, to Atampy26 automatically to help fix this issue. If you choose
		not to, you can find it in <code>%appdata%\app.glacierkit</code>.
		<pre class="mt-2 p-4 bg-neutral-800 overflow-x-auto"><code>{errorModalError}</code></pre>
	</ModalBody>
	<ModalFooter
		primaryButtonText="Send crash report"
		secondaryButtonText="Continue without sending report"
		on:click:button--secondary={async () => {
			lastPanicModalOpen = false

			await event({
				type: "global",
				data: {
					type: "clearLastPanic"
				}
			})
		}}
	/>
</ComposedModal>

<ComposedModal
	open={logUploadRejectedModalOpen}
	on:submit={() => {
		logUploadRejectedModalOpen = false
	}}
>
	<ModalHeader title="Upload failed" />
	<ModalBody>
		The file is likely too large to be uploaded automatically. You can find it in <code>%appdata%\app.glacierkit</code>; please send it to Atampy26 via Discord.
	</ModalBody>
	<ModalFooter primaryButtonText="OK" />
</ComposedModal>

<ComposedModal
	open={updateModalOpen}
	on:submit={async () => {
		updateModalOpen = false

		await installUpdate()
		await relaunch()
	}}
>
	<ModalHeader title="Update available to version {updateManifest.version}" />
	<ModalBody>
		Changes made since the currently installed version:
		<ul class="changelog mt-1">
			{#each commitsSinceLastVersion as commit}
				<li>{commit}</li>
			{/each}
		</ul>
	</ModalBody>
	<ModalFooter
		primaryButtonText="Install update"
		secondaryButtonText="Not now"
		on:click:button--secondary={() => {
			updateModalOpen = false
		}}
	/>
</ComposedModal>

<header data-tauri-drag-region class:bx--header={true}>
	<SkipToContent />

	<!-- svelte-ignore a11y-missing-attribute -->
	<a data-tauri-drag-region class:bx--header__name={true} use:help={{ title: "GlacierKit title", description: "This is in fact the app you are using." }}
		>GlacierKit<span class="font-normal ml-1">{#await getVersion() then x}{x}{/await}</span></a
	>

	<div data-tauri-drag-region class="pointer-events-none cursor-none w-full text-center text-neutral-400">{windowTitle}</div>

	<div data-tauri-drag-region class="flex flex-row items-center justify-end text-white">
		<div
			class="h-full p-3.5 hover:bg-neutral-700 active:bg-neutral-600"
			on:click={() => {
				helpRayActive = !helpRayActive
				if (helpRayActive) {
					trackEvent("Activate help ray")
				}
			}}
		>
			<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="size-5">
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M9.879 7.519c1.171-1.025 3.071-1.025 4.242 0 1.172 1.025 1.172 2.687 0 3.712-.203.179-.43.326-.67.442-.745.361-1.45.999-1.45 1.827v.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9 5.25h.008v.008H12v-.008Z"
				/>
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.minimize} use:help={{ title: "Minimise", description: "Minimise the application." }}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M18 12H6" />
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-neutral-700 active:bg-neutral-600" on:click={appWindow.toggleMaximize} use:help={{ title: "Maximise", description: "Maximise the application." }}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					d="M16.5 8.25V6a2.25 2.25 0 00-2.25-2.25H6A2.25 2.25 0 003.75 6v8.25A2.25 2.25 0 006 16.5h2.25m8.25-8.25H18a2.25 2.25 0 012.25 2.25V18A2.25 2.25 0 0118 20.25h-7.5A2.25 2.25 0 018.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 00-2.25 2.25v6"
				/>
			</svg>
		</div>
		<div class="h-full p-4 hover:bg-red-600 active:bg-red-700" on:click={appWindow.close} use:help={{ title: "Close", description: "Close the application." }}>
			<svg fill="none" stroke="currentColor" width="16px" stroke-width="1.5" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
				<path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
			</svg>
		</div>
	</div>
</header>

<div class="w-full h-mid">
	<slot />
</div>

<div class="h-6 flex items-center gap-4 px-3 bg-neutral-600" use:help={{ title: "Task bar", description: "You can see all currently running background tasks here." }}>
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

<HelpRay bind:enabled={helpRayActive} />

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
		@apply text-yellow-200;
	}

	:global(.colourblind-mode .jstree-default .jstree-search) {
		@apply font-bold;
	}

	:global(.jstree-default .item-new > a) {
		@apply text-emerald-200 !important;
	}

	:global(.colourblind-mode .jstree-default .item-new > a) {
		@apply font-bold !important;
	}

	:global(.jstree-default .item-modified > a) {
		@apply text-purple-200 !important;
	}

	:global(.colourblind-mode .jstree-default .item-modified > a) {
		@apply italic !important;
	}

	:global(.jstree-default .item-removed > a) {
		@apply text-red-200 !important;
	}

	:global(.colourblind-mode .jstree-default .item-removed > a) {
		@apply line-through !important;
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

	:global(.no-menu-spacing .bx--list-box__menu-item__option) {
		padding-right: 0;
		margin-left: 0.75rem;
		margin-right: 0.5rem;
	}

	:global(.changelog li) {
		list-style-position: inside;
		list-style-type: disclosure-closed;
	}
</style>
