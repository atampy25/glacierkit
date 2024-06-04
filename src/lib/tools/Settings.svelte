<script lang="ts">
	import { event } from "$lib/utils"
	import type { GameInstall, SettingsRequest } from "$lib/bindings-types"
	import { Checkbox, TooltipIcon } from "carbon-components-svelte"
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
				selectedGameInstall = request.data.settings.gameInstall || null
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

	let extractModdedFiles = false
	let colourblind = false

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

	<p class="mt-1">Game</p>
	<div class="mt-1 flex flex-wrap gap-2">
		{#each gameInstalls as gameInstall}
			<div
				class="bg-neutral-900 p-4 flex items-center justify-center border-solid border-neutral-300 cursor-pointer"
				class:border-2={selectedGameInstall === gameInstall.path}
				on:click={async () => {
					selectedGameInstall = gameInstall.path

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
					<span class="break-all">{gameInstall.path}</span>
				</div>
			</div>
		{/each}
		<div
			class="bg-neutral-900 p-4 flex items-center justify-center border-solid border-neutral-300 cursor-pointer"
			class:border-2={selectedGameInstall === null}
			on:click={async () => {
				selectedGameInstall = null

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
		</div>
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
