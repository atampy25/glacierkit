<script lang="ts">
	import type { ResourceChangelogEntry, ResourceOverviewData, ResourceOverviewRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import {
		Accordion,
		AccordionItem,
		Button,
		ButtonSet,
		ClickableTile,
		ContentSwitcher,
		DataTable,
		ExpandableTile,
		ImageLoader,
		ListItem,
		OrderedList,
		StructuredList,
		StructuredListBody,
		StructuredListCell,
		StructuredListRow,
		Switch,
		Table,
		TableBody,
		TableCell,
		TableHead,
		TableHeader,
		TableRow,
		Tile
	} from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Edit from "carbon-icons-svelte/lib/Edit.svelte"
	import DocumentExport from "carbon-icons-svelte/lib/DocumentExport.svelte"
	import { trackEvent } from "@aptabase/tauri"
	import { convertFileSrc } from "@tauri-apps/api/tauri"
	import WaveformPlayer from "$lib/components/WaveformPlayer.svelte"
	import MultiWaveformPlayer from "$lib/components/MultiWaveformPlayer.svelte"
	import Monaco from "./Monaco.svelte"
	import { v4 } from "uuid"
	import { help } from "$lib/helpray"
	import MeshPreview from "$lib/components/MeshPreview.svelte"
	import { Pane, Splitpanes } from "svelte-splitpanes"
	import { ColumnDependency, IbmDataProductExchange, SoftwareResource, TrashCan } from "carbon-icons-svelte"
	import AddLarge from "carbon-icons-svelte/lib/AddLarge.svelte"
	import { platform } from "@tauri-apps/api/os"

	export let id: string

	let hash = ""
	let filetype = ""
	let partition = ""
	let pathOrHint: string | null = null
	let dependencies: [string, string, string | null, string, boolean][] = []
	let reverseDependencies: [string, string, string | null][] = []
	let changelog: ResourceChangelogEntry[] = []
	let data: ResourceOverviewData | null = null

	let previewImage: any = null
	let referenceTab = 0

	const typesWithPreview = ["Image", "Mesh", "Audio", "MultiAudio", "GenericRL", "Ores", "Json", "HMLanguages", "LocalisedLine", "Material"]

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
				partition = request.data.chunk_patch.replace(/patch[0-9]+/, "")
				pathOrHint = request.data.path_or_hint
				dependencies = request.data.dependencies
				reverseDependencies = request.data.reverse_dependencies
				changelog = request.data.changelog
				data = request.data.data
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}
</script>

<div
	class="w-full h-full max-h-full flex flex-col p-4"
	use:help={{
		title: "Resource overview",
		description: "The resource overview shows basic information about (and potentially previews of) game resources, and lets you perform actions like extracting them in different formats."
	}}
>
	{#if data}
		<div class="text-2xl mb-3 font-bold break-all">
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
				<div>Partition</div>
				<div class="text-xl">{partition}</div>
			</div>
		</div>

		<div style="height: calc(100vh - 18rem)">
			<Splitpanes theme="">
				<Pane minSize={50} class="h-full">
					<div class="h-full overflow-y-auto pr-2">
						{#if typesWithPreview.includes(data.type)}
							<div
								class="mb-2"
								use:help={{
									title: "Preview",
									description: "A preview of the resource."
								}}
							>
								<Tile>
									<h4 class="mb-1">Preview</h4>
									{#if data.type === "Image"}
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
											class="h-[30vh] w-fit bg-[#7f7f7f] aspect-square"
											style="image-rendering: pixelated"
											bind:this={previewImage}
											on:load={() => {
												previewImage = previewImage
											}}
											src={convertFileSrc(data.data.image_path)}
											alt="Resource preview"
										/>
									{:else if data.type === "Mesh"}
										<div class="h-[30vh]">
											<MeshPreview obj={data.data.obj} boundingBox={data.data.bounding_box} />
										</div>
									{:else if data.type === "Audio"}
										{#await platform() then platform}
											{#if platform === "linux"}
												<div class="text-neutral-400">Audio preview is unavailable on Linux due to a bug in WebKit.</div>
											{:else}
												<WaveformPlayer src={convertFileSrc(data.data.wav_path)} />
											{/if}
										{/await}
									{:else if data.type === "MultiAudio"}
										{#await platform() then platform}
											{#if platform === "linux"}
												<div class="text-neutral-400">Audio preview is unavailable on Linux due to a bug in WebKit.</div>
											{:else}
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
											{/if}
										{/await}
									{:else if data.type === "GenericRL" || data.type === "Ores" || data.type === "Json" || data.type === "HMLanguages" || data.type === "Material"}
										<div class="h-[30vh]">
											<Monaco id={v4()} content={data.data.json} />
										</div>
									{:else if data.type === "LocalisedLine"}
										<div class="max-h-[30vh] overflow-y-auto">
											<DataTable
												headers={[
													{ key: "lang", value: "Language", width: "8rem" },
													{ key: "val", value: "String" }
												]}
												rows={data.data.languages.map(([lang, val], ind) => ({ id: ind, lang, val }))}
											/>
										</div>
									{/if}
								</Tile>
							</div>
						{/if}
						<div
							class="mb-2"
							use:help={{
								title: "Actions",
								description: "Actions you can perform on the resource."
							}}
						>
							<Tile>
								<h4 class="mb-2">Actions</h4>
								<div class="flex flex-wrap gap-2">
									{#if data.type === "Entity"}
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
									{:else if data.type === "Image"}
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
									{:else if data.type === "Audio"}
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
									{:else if data.type === "MultiAudio"}
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
												trackEvent("Extract WWEV file as original")

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
									{:else if data.type === "GenericRL"}
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
												trackEvent("Extract generic ResourceLib file as binary")

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
									{:else if data.type === "Ores"}
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
									{:else if data.type === "Repository"}
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
									{:else if data.type === "Unlockables"}
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
									{:else if data.type === "HMLanguages"}
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
												trackEvent("Extract HMLanguages file as binary", { hash, filetype })

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
									{:else if data.type === "Mesh"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract mesh file as original")

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
									{:else if data.type === "Material"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract material file as original")

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
									{:else if data.type === "Json" || data.type === "LocalisedLine" || data.type === "Generic"}
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
									{/if}
								</div>
							</Tile>
						</div>
						<div
							use:help={{
								title: "History",
								description: "A log of changes made to the resource in each patch, in chronological order from top to bottom."
							}}
						>
							<Tile>
								<h4 class="mb-2">History</h4>
								<Table size="medium">
									<TableHead>
										<TableRow>
											<TableHeader class="w-5"></TableHeader>
											<TableHeader>Partition</TableHeader>
											<TableHeader class="w-15">Patch</TableHeader>
											<TableHeader>Action</TableHeader>
										</TableRow>
									</TableHead>
									<TableBody>
										{#each changelog as event}
											<TableRow>
												<TableCell>
													{#if event.operation == "Init"}
														<AddLarge title="Added" />
													{:else if event.operation == "Edit"}
														<SoftwareResource title="Modified" />
													{:else if event.operation == "Delete"}
														<TrashCan title="Removed" />
													{/if}
												</TableCell>
												<TableCell>{event.partition}</TableCell>
												<TableCell>{event.patch}</TableCell>
												<TableCell>{event.description}</TableCell>
											</TableRow>
										{/each}
									</TableBody>
								</Table>
							</Tile>
						</div>
					</div>
				</Pane>
				<Pane size={45} class="h-full flex flex-col">
					<ContentSwitcher class="h-10 pb-2" bind:selectedIndex={referenceTab}>
						<Switch>
							<div class="flex items-center gap-2">
								<ColumnDependency class="flex-shrink-0" />
								<div class="truncate">References</div>
							</div>
						</Switch>
						<Switch>
							<div class="flex items-center gap-2">
								<ColumnDependency class="flex-shrink-0 -scale-x-100" />
								<div class="truncate">Reverse references</div>
							</div>
						</Switch>
					</ContentSwitcher>
					{#if referenceTab == 0}
						<div
							class="h-full overflow-y-auto pr-2 flex flex-col gap-2"
							use:help={{ title: "References", description: "Other resources that this resource depends on, listed in the order stored in the game files." }}
						>
							{#each dependencies as [hash, type, path, flag, inGame]}
								{#if type}
									<ClickableTile
										style="min-height: unset"
										on:click={async (e) => {
											trackEvent(`Follow reference ${e.ctrlKey ? "in new tab " : "from resource overview"}`)

											await event({
												type: "editor",
												data: {
													type: "resourceOverview",
													data: !e.ctrlKey
														? {
																type: "followDependency",
																data: {
																	id,
																	new_hash: hash
																}
															}
														: {
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
									</ClickableTile>
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
					{/if}
					{#if referenceTab == 1}
						<div
							class="h-full overflow-y-auto pr-2 flex flex-col gap-2"
							use:help={{ title: "Reverse references", description: "Other resources that depend upon this resource, sorted alphabetically." }}
						>
							{#each reverseDependencies as [hash, type, path]}
								{#if type}
									<ClickableTile
										style="min-height: unset"
										on:click={async (e) => {
											trackEvent(`Follow reverse reference ${e.ctrlKey ? "in new tab " : "from resource overview"}`)

											await event({
												type: "editor",
												data: {
													type: "resourceOverview",
													data: !e.ctrlKey
														? {
																type: "followDependency",
																data: {
																	id,
																	new_hash: hash
																}
															}
														: {
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
									</ClickableTile>
								{:else}
									<div class="bg-[#303030] p-3">
										<div class="font-bold text-base -mt-1">{hash}</div>
										<div class="break-all">Unknown resource</div>
									</div>
								{/if}
							{/each}
						</div>
					{/if}
				</Pane>
			</Splitpanes>
		</div>
	{:else}
		Loading...
	{/if}
</div>
