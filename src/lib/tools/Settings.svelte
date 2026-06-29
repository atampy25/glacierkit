<script lang="ts">
	import { event } from "$lib/utils"
	import type { GameInstall, SettingsRequest } from "$lib/bindings"
	import { Checkbox, TooltipIcon, TileGroup, RadioTile, Button } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Information from "carbon-icons-svelte/lib/Information.svelte"
	import Close from "carbon-icons-svelte/lib/Close.svelte"
	import ListEditor from "$lib/components/ListEditor.svelte"
	import { help } from "$lib/helpray"
	import { open } from "@tauri-apps/plugin-dialog"

	export async function handleRequest(request: SettingsRequest) {
		console.log("Settings tool handling request", request)

		switch (request.type) {
			case "initialise":
				gameInstalls = request.data.gameInstalls
				extractModdedFiles = request.data.settings.extractModdedFiles
				colourblind = request.data.settings.colourblindMode
				editorConnectionEnabled = request.data.settings.editorConnection
				selectedGameInstall = request.data.settings.gamePath
				customGamePaths = request.data.settings.customGamePaths
				break

			case "changeProjectSettings":
				projectLoaded = true
				customPaths = request.data.settings.customPaths
				break

			default:
				request satisfies never
				break
		}
	}

	onMount(async () => {
		await event({
			type: "tool",
			data: {
				type: "settings",
				data: {
					type: "initialise"
				}
			}
		})
	})

	async function changeExtractModdedFiles({ target }: { target: EventTarget | null }) {
		if (target) {
			const _target = target as HTMLInputElement

			extractModdedFiles = _target.checked
			await event({
				type: "tool",
				data: {
					type: "settings",
					data: {
						type: "changeExtractModdedFiles",
						data: { value: _target.checked }
					}
				}
			})
		}
	}

	async function changeColourblind({ target }: { target: EventTarget | null }) {
		if (target) {
			const _target = target as HTMLInputElement

			colourblind = _target.checked
			await event({
				type: "tool",
				data: {
					type: "settings",
					data: {
						type: "changeColourblindMode",
						data: { value: _target.checked }
					}
				}
			})
		}
	}

	async function changeEditorConnectionEnabled({ target }: { target: EventTarget | null }) {
		if (target) {
			const _target = target as HTMLInputElement

			editorConnectionEnabled = _target.checked
			await event({
				type: "tool",
				data: {
					type: "settings",
					data: {
						type: "changeEditorConnection",
						data: { value: _target.checked }
					}
				}
			})
		}
	}

	let extractModdedFiles = false
	let colourblind = false
	let editorConnectionEnabled = true

	let projectLoaded = false

	let gameInstalls: GameInstall[] = []
	let selectedGameInstall: string | null = null
	let customGamePaths: string[] = []

	$: if (colourblind) {
		document.body.classList.add("colourblind-mode")
	} else {
		document.body.classList.remove("colourblind-mode")
	}

	let customPaths: string[] = []
</script>

<div
	class="w-full h-full p-6 overflow-x-hidden overflow-y-auto"
	use:help={{ title: "Settings", description: "This panel lets you modify GlacierKit's settings. Some settings are GlacierKit-wide, while others are project-specific." }}
>
	<h4>GlacierKit settings</h4>
	<div class="flex items-center gap-2">
		<div class="flex-shrink">
			<Checkbox checked={extractModdedFiles} on:change={changeExtractModdedFiles} labelText="Allow extracting modded files" />
		</div>
		<TooltipIcon icon={Information}>
			<span slot="tooltipText" style="font-size: 0.875rem; margin-top: 0.5rem; margin-bottom: 0.5rem">
				GlacierKit usually ignores modded copies of files (files past chunk0patch9) when reading game files.
			</span>
		</TooltipIcon>
	</div>
	<div class="flex items-center gap-2">
		<div class="flex-shrink">
			<Checkbox checked={colourblind} on:change={changeColourblind} labelText="Use non-colour contrast" />
		</div>
		<TooltipIcon icon={Information}>
			<span slot="tooltipText" style="font-size: 0.875rem; margin-top: 0.5rem; margin-bottom: 0.5rem">
				Will use text features like italics and strikethrough in addition to colour to mark contrast.
			</span>
		</TooltipIcon>
	</div>
	<div class="flex items-center gap-2">
		<div class="flex-shrink">
			<Checkbox checked={editorConnectionEnabled} on:change={changeEditorConnectionEnabled} labelText="Enable editor connection" />
		</div>
		<TooltipIcon icon={Information}>
			<span slot="tooltipText" style="font-size: 0.875rem; margin-top: 0.5rem; margin-bottom: 0.5rem">
				By default, GlacierKit connects automatically to the SDK editor and syncs any changes you make. If you don't want this, you can disable the editor connection.
			</span>
		</TooltipIcon>
	</div>

	<p class="mt-1">Game</p>
	<div class="mt-1 flex flex-wrap gap-2">
		<TileGroup
			selected={selectedGameInstall || "null"}
			on:select={async ({ detail }) => {
				if (detail === "custom") return
				selectedGameInstall = detail === "null" ? null : detail
				await event({
					type: "tool",
					data: {
						type: "settings",
						data: {
							type: "changeGameInstall",
							data: { path: selectedGameInstall }
						}
					}
				})
			}}
		>
			{#each gameInstalls as install (install.path)}
				<RadioTile value={install.path}>
					<div class="relative">
						<div class="font-bold mb-2">
							{{
								h1: "HITMAN™",
								h2: "HITMAN 2",
								h3: "HITMAN 3",
								fl: "007 First Light"
							}[install.version]} ({{
								steam: "Steam",
								epic: "Epic Games",
								microsoft: "Microsoft",
								gog: "GOG"
							}[install.platform]})
						</div>
						<span class="break-all">{install.path}</span>

						<div class="absolute -bottom-1 -right-10">
							{#if customGamePaths.includes(install.path)}
								<Button
									size="small"
									kind="ghost"
									icon={Close}
									iconDescription="Remove"
									on:click={async () => {
										await event({
											type: "tool",
											data: {
												type: "settings",
												data: {
													type: "removeCustomGamePath",
													data: { path: install.path }
												}
											}
										})
									}}
								/>
							{/if}
						</div>
					</div>
				</RadioTile>
			{/each}
			<RadioTile
				value="custom"
				on:click={async () => {
					const folder = await open({
						title: "Select Retail folder",
						directory: true
					})

					if (folder) {
						await event({
							type: "tool",
							data: {
								type: "settings",
								data: {
									type: "changeGameInstall",
									data: { path: folder as string }
								}
							}
						})
					} else {
						selectedGameInstall = null
					}
				}}>Select game path</RadioTile
			>
			<RadioTile value="null">No game</RadioTile>
		</TileGroup>
	</div>

	<h4 class="mt-4">Project settings</h4>
	{#if projectLoaded}
		<p class="mt-1 mb-1">Custom paths</p>
		<ListEditor
			bind:data={customPaths}
			on:updated={async ({ detail }) => {
				await event({
					type: "tool",
					data: {
						type: "settings",
						data: {
							type: "changeCustomPaths",
							data: { value: detail }
						}
					}
				})
			}}
		/>
	{:else}
		<p>No project loaded</p>
	{/if}
</div>
