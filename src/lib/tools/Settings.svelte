<script lang="ts">
	import { event } from "$lib/utils"
	import type { GameInstall, SettingsRequest } from "$lib/bindings-types"
	import { Checkbox, TooltipIcon } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Information from "carbon-icons-svelte/lib/Information.svelte"

	export async function handleRequest(request: SettingsRequest) {
		console.log("Settings tool handling request", request)

		switch (request.type) {
			case "initialise":
				gameInstalls = request.data.game_installs
				extractModdedFiles = request.data.settings.extractModdedFiles
				selectedGameInstall = request.data.settings.gameInstall || null
				break

			case "changeProjectSettings":
				projectLoaded = true
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

	let extractModdedFiles = false

	let projectLoaded = false

	let gameInstalls: GameInstall[] = []
	let selectedGameInstall: string | null = null
</script>

<div class="w-full h-full p-6 overflow-x-hidden overflow-y-auto">
	<h4>Deeznuts settings</h4>
	<div class="flex items-center gap-2">
		<div class="flex-shrink">
			<Checkbox checked={extractModdedFiles} on:change={changeExtractModdedFiles} labelText="Allow extracting modded files" />
		</div>
		<TooltipIcon icon={Information}>
			<span slot="tooltipText" style="font-size: 0.875rem; margin-top: 0.5rem; margin-bottom: 0.5rem">
				Deeznuts usually ignores modded copies of files (files past chunk0patch9) when reading game files.
			</span>
		</TooltipIcon>
	</div>
	
	<p>Game</p>
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
					<span>{gameInstall.path}</span>
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
		<p>There are no project settings (yet)</p>
	{:else}
		<p>No project loaded</p>
	{/if}
</div>
