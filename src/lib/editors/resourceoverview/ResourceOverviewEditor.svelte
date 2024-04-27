<script lang="ts">
	import type { ResourceOverviewData, ResourceOverviewRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { Button, DataTable } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Edit from "carbon-icons-svelte/lib/Edit.svelte"
	import DocumentExport from "carbon-icons-svelte/lib/DocumentExport.svelte"
	import { trackEvent } from "@aptabase/tauri"
	import { convertFileSrc } from "@tauri-apps/api/tauri"
	import WaveformPlayer from "$lib/components/WaveformPlayer.svelte"
	import MultiWaveformPlayer from "$lib/components/MultiWaveformPlayer.svelte"
	import Monaco from "./Monaco.svelte"
	import { v4 } from "uuid"

	export let id: string

	let hash = ""
	let filetype = ""
	let chunk = ""
	let pathOrHint: string | null = null
	let dependencies: [string, string, string | null, string, boolean][] = []
	let reverseDependencies: [string, string, string | null][] = []
	let data: ResourceOverviewData | null = null

	let previewImage: any = null

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				type: "resourceOverview",
				data: {
					type: "initialise",
					data: {
						id
					}
				}
			}
		})
	})

	export async function handleRequest(request: ResourceOverviewRequest) {
		console.log(`Resource overview ${id} handling request`, request)

		switch (request.type) {
			case "initialise":
				hash = request.data.hash
				filetype = request.data.filetype
				chunk = request.data.chunk_patch.replace(/patch[0-9]+/, "")
				pathOrHint = request.data.path_or_hint
				dependencies = request.data.dependencies
				reverseDependencies = request.data.reverse_dependencies
				data = request.data.data
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}
</script>

<div class="w-full h-full flex flex-col p-4 overflow-y-auto">
	{#if data}
		{#if data.type === "Entity"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Factory</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Blueprint</div>
					<div class="text-xl">{data.data.blueprint_hash}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={Edit}
					on:click={async () => {
						trackEvent("Open QN entity in editor from resource overview")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "openInEditor",
									data: {
										id
									}
								}
							}
						})
					}}>Open in editor</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract entity to QN JSON")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsQN",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as QuickEntity JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract TEMP as binary file")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract TEMP as binary file</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract TEMP as RL JSON")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractTEMPAsRT",
									data: {
										id
									}
								}
							}
						})
					}}>Extract TEMP as ResourceLib JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract TBLU as binary file")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractTBLUAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract TBLU as binary file</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract TBLU as RL JSON")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractTBLUAsRT",
									data: {
										id
									}
								}
							}
						})
					}}>Extract TBLU as ResourceLib JSON</Button
				>
			</div>
		{:else if data.type === "Image"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			{#if previewImage}
				<div class="text-neutral-400 mb-2 flex items-center gap-4">
					<span>Resolution: {previewImage.naturalWidth}x{previewImage.naturalHeight}</span>
					{#if data.data.dds_data}
						<span>Type: {data.data.dds_data[0]}</span>
						<span>Format: {data.data.dds_data[1]}</span>
					{/if}
				</div>
			{/if}
			<img
				class="mb-4 h-[30vh] w-fit bg-[#7f7f7f]"
				style="image-rendering: pixelated"
				bind:this={previewImage}
				on:load={() => {
					previewImage = previewImage
				}}
				src={convertFileSrc(data.data.image_path)}
				alt="Resource preview"
			/>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						// Analytics tracked on Rust end

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsImage",
									data: {
										id
									}
								}
							}
						})
					}}>Extract image</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract image file as original")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "Audio"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4">
				<WaveformPlayer src={convertFileSrc(data.data.wav_path)} />
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract audio file as WAV")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsWav",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as WAV</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract audio file as original")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "MultiAudio"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4">
				<div class="text-neutral-400 mb-2">{data.data.name}</div>
				{#if data.data.wav_paths.length}
					<MultiWaveformPlayer
						src={data.data.wav_paths.map((a) => [a[0], convertFileSrc(a[1])])}
						on:download={async ({ detail }) => {
							trackEvent("Extract specific audio from WWEV file as WAV")

							await event({
								type: "editor",
								data: {
									type: "resourceOverview",
									data: {
										type: "extractSpecificMultiWav",
										data: {
											id,
											index: detail
										}
									}
								}
							})
						}}
					/>
				{:else}
					<div class="-mt-1 text-lg">No linked audio</div>
				{/if}
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract WWEV file as WAVs")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractMultiWav",
									data: {
										id
									}
								}
							}
						})
					}}>Extract all as WAVs</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract audio file as original")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "GenericRL"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4 h-[30vh]">
				<Monaco id={v4()} content={data.data.json} />
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract generic ResourceLib file as JSON", { hash, filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsRTGeneric",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as ResourceLib JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract generic ResourceLib file as binary", { hash, filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "Ores"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4 h-[30vh]">
				<Monaco id={v4()} content={data.data.json} />
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract ORES as JSON")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractORESAsJson",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract ORES as binary")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "Repository"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={Edit}
					on:click={async () => {
						trackEvent("Open repository in editor from resource overview")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "openInEditor",
									data: {
										id
									}
								}
							}
						})
					}}>Open in editor</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract repository to file")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "Unlockables"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={Edit}
					on:click={async () => {
						trackEvent("Open unlockables in editor from resource overview")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "openInEditor",
									data: {
										id
									}
								}
							}
						})
					}}>Open in editor</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract unlockables as JSON")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractORESAsJson",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract unlockables as binary")

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "Json"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4 h-[30vh]">
				<Monaco id={v4()} content={data.data.json} />
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract generic file", { hash, filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "HMLanguages"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4 h-[30vh]">
				<Monaco id={v4()} content={data.data.json} />
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract HMLanguages file as JSON", { filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsHMLanguages",
									data: {
										id
									}
								}
							}
						})
					}}>Extract as JSON</Button
				>
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract HMLanguages file as binary", { filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else if data.type === "LocalisedLine"}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Preview</h4>
			<div class="mb-4 w-[30rem] max-h-[30vh] overflow-y-auto">
				<DataTable
					headers={[
						{ key: "lang", value: "Language", width: "8rem" },
						{ key: "val", value: "String" }
					]}
					rows={data.data.languages.map(([lang, val], ind) => ({ id: ind, lang, val }))}
				/>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract generic file", { hash, filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{:else}
			<div class="text-2xl mb-2 font-bold break-all">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Type</div>
					<div class="text-xl">{filetype}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={DocumentExport}
					on:click={async () => {
						trackEvent("Extract generic file", { hash, filetype })

						await event({
							type: "editor",
							data: {
								type: "resourceOverview",
								data: {
									type: "extractAsFile",
									data: {
										id
									}
								}
							}
						})
					}}>Extract file</Button
				>
			</div>
		{/if}

		<div class="grid grid-cols-2 gap-2 flex-grow basis-0">
			<div class="flex flex-col">
				<h4 class="mb-1">Dependencies</h4>
				<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
					{#each dependencies as [hash, type, path, flag, inGame]}
						{#if type}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
									trackEvent("Follow dependency from resource overview")

									await event({
										type: "editor",
										data: {
											type: "resourceOverview",
											data: {
												type: "followDependency",
												data: {
													id,
													new_hash: hash
												}
											}
										}
									})
								}}
								on:contextmenu={async (e) => {
									e.preventDefault()
									trackEvent("Follow dependency in new tab from resource overview")

									await event({
										type: "editor",
										data: {
											type: "resourceOverview",
											data: {
												type: "followDependencyInNewTab",
												data: {
													id,
													hash
												}
											}
										}
									})
								}}
							>
								<div class="text-base -mt-1"
									><span class="font-bold">{hash}.{type}</span>
									{flag}</div
								>
								<div class="break-all">{path || "No path"}</div>
								{#if !inGame}
									<div class="text-base">Not present in game files</div>
								{/if}
							</div>
						{:else}
							<div class="bg-[#303030] p-3">
								<div class="text-base -mt-1"
									><span class="font-bold">{hash}</span>
									{flag}</div
								>
								<div class="break-all">Unknown resource</div>
								{#if !inGame}
									<div class="text-base">Not present in game files</div>
								{/if}
							</div>
						{/if}
					{/each}
				</div>
			</div>
			<div class="flex flex-col">
				<h4 class="mb-1">Reverse dependencies</h4>
				<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
					{#each reverseDependencies as [hash, type, path]}
						{#if type}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
									trackEvent("Follow reverse dependency from resource overview")

									await event({
										type: "editor",
										data: {
											type: "resourceOverview",
											data: {
												type: "followDependency",
												data: {
													id,
													new_hash: hash
												}
											}
										}
									})
								}}
								on:contextmenu={async (e) => {
									e.preventDefault()
									trackEvent("Follow reverse dependency in new tab from resource overview")

									await event({
										type: "editor",
										data: {
											type: "resourceOverview",
											data: {
												type: "followDependencyInNewTab",
												data: {
													id,
													hash
												}
											}
										}
									})
								}}
							>
								<div class="font-bold text-base -mt-1"
									>{hash}{#if type}.{type}{/if}</div
								>
								<div class="break-all">{path || "No path"}</div>
							</div>
						{:else}
							<div class="bg-[#303030] p-3">
								<div class="font-bold text-base -mt-1">{hash}</div>
								<div class="break-all">Unknown resource</div>
							</div>
						{/if}
					{/each}
				</div>
			</div>
		</div>
	{:else}
		Loading...
	{/if}
</div>
