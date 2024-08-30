<script lang="ts">
	import { event } from "$lib/utils"
	import type { GameInstall, SettingsRequest } from "$lib/bindings-types"
	import { Checkbox, RadioTile, TileGroup, TooltipIcon } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Information from "carbon-icons-svelte/lib/Information.svelte"
	import ListEditor from "$lib/components/ListEditor.svelte"
	import { help } from "$lib/helpray"

	export async function handleRequest(request: SettingsRequest) {
		console.log("Settings tool handling request", request)

		switch (request.type) {
			case "initialise":
				gameInstalls = request.data.game_installs
				extractModdedFiles = request.data.settings.extractModdedFiles
				colourblind = request.data.settings.colourblindMode
				editorConnectionEnabled = request.data.settings.editorConnection
				selectedGameInstall = request.data.settings.gameInstall || "none"
				break

			case "changeProjectSettings":
				projectLoaded = true
				customPaths = request.data.customPaths
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
						data: _target.checked
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
						type: "changeColourblind",
						data: _target.checked
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
						data: _target.checked
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

	<TileGroup class="mt-1 flex flex-wrap gap-2" name="Game"
		on:select={({ detail }) => (selectedGameInstall = detail)}>

		{#each gameInstalls as gameInstall}
			<RadioTile 
				value={ gameInstall.path }
				class="p-4 flex items-center justify-center"
				checked={selectedGameInstall === gameInstall.path}
				on:click={async () => {
					await event({
						type: "tool",
						data: {
							type: "settings",
							data: {
								type: "changeGameInstall",
								data: gameInstall.path
							}
						}
					})
				}}
			>
				<div>
					<div class="font-bold mb-2">{gameInstall.version === "h1" ? "HITMAN™" : gameInstall.version === "h2" ? "HITMAN 2" : "HITMAN 3"} ({gameInstall.platform})</div>
					<span class="break-all text-sm text-slate-300">{gameInstall.path}</span>
				</div>
			</RadioTile>
		{/each}
		<RadioTile
			value={"none"}
			class="p-4 flex items-center justify-center"
			checked={selectedGameInstall === "none"}
			on:click={async () => {

				await event({
					type: "tool",
					data: {
						type: "settings",
						data: {
							type: "changeGameInstall",
							data: null
						}
					}
				})
			}}
		>
			<p>No game</p>
		</RadioTile>
	</TileGroup>

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
							data: detail
						}
					}
				})
			}}
		/>
	{:else}
		<p>No project loaded</p>
	{/if}
</div>
