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
	let previewAvailable: boolean = true

	function editorAvailable(type: string): boolean {
		if (type === "Entity" || type === "Repository" || type === "Unlockables") {
			return true
		} else return false
	}

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
	class="w-full h-full flex flex-col p-4"
	use:help={{
		title: "Resource overview",
		description: "The resource overview shows basic information about (and potentially previews of) game resources, and lets you perform actions like extracting them in different formats."
	}}
>
	{#if data}
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
				<div>Partition</div>
				<div class="text-xl">{partition}</div>
			</div>
		</div>

		<Splitpanes theme="" class="h-full flex-auto overflow-auto">
			<Pane minSize={50} class="h-full flex flex-col pr-1 ">
				<div class="overflow-y-auto no-scrollbar" >
					<div class="pb-2" hidden={!previewAvailable}>
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
									class="mb-4 h-[30vh] w-fit bg-[#7f7f7f] aspect-square"
									style="image-rendering: pixelated"
									bind:this={previewImage}
									on:load={() => {
										previewImage = previewImage
									}}
									src={convertFileSrc(data.data.image_path)}
									alt="Resource preview"
								/>
							{:else if data.type === "Mesh"}
								<div class="mb-4 h-[30vh]">
									<MeshPreview obj={data.data.obj} boundingBox={data.data.bounding_box} />
								</div>
							{:else if data.type === "Audio"}
								<div class="mb-4">
									<WaveformPlayer src={convertFileSrc(data.data.wav_path)} />
								</div>
							{:else if data.type === "MultiAudio"}
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
							{:else if data.type === "GenericRL" || data.type === "Ores" || data.type === "Json" || data.type === "HMLanguages"}
								<div class="mb-4 h-[30vh]">
									<Monaco id={v4()} content={data.data.json} />
								</div>
							{:else if data.type === "LocalisedLine"}
								<div class="mb-4 max-h-[30vh] overflow-y-auto">
									<DataTable
										headers={[
											{ key: "lang", value: "Language", width: "8rem" },
											{ key: "val", value: "String" }
										]}
										rows={data.data.languages.map(([lang, val], ind) => ({ id: ind, lang, val }))}
									/>
								</div>
							{:else}
								{(previewAvailable = false)}
							{/if}
						</Tile>
					</div>
					<div class="pb-2">
					<Tile>
						<h4 class="mb-2 pb-2">Actions</h4>
						<div class="flex flex-wrap gap-2 mb-4">
							{#if editorAvailable(data.type)}
								<Button
									icon={Edit}
									on:click={async () => {
										trackEvent(`Open ${data?.type ?? "data"} in editor from resource overview`)
	
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
							{/if}
							{#if data.type === "Entity"}
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
							{:else if data.type === "Unlockables"}
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
									}}>Export to JSON</Button
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
									}}>Export to JSON</Button
								>
							{/if}
							<Button
								kind="tertiary"
								icon={DocumentExport}
								on:click={async () => {
									trackEvent(`Extract ${data?.type ?? "generic"} file`, { hash, filetype })
	
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
					</Tile>
					</div>
					<div class="pb-2">
						<ExpandableTile>
							<div slot="above">
								<h4 class="mb-2 pb-2">History</h4>
							</div>
							<div slot="below">
								<Table class="mb-7" size="medium">
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
													<AddLarge/>
												{:else if event.operation == "Edit"}
													<SoftwareResource/>
												{:else if event.operation == "Delete"}
													<TrashCan/>
												{/if}
												
											</TableCell>
											<TableCell>{event.partition}</TableCell>
											<TableCell>{event.patch}</TableCell>
											<TableCell>{event.description}</TableCell>
										</TableRow>
										{/each}
									</TableBody>
								</Table>
							</div>
						</ExpandableTile>
					</div>
				</div>
			</Pane>
			<Pane size={45} class="h-full flex flex-col">
				<ContentSwitcher class="h-10 pb-2" bind:selectedIndex={referenceTab}>
					<Switch>
						<div style="display: flex; align-items: left;">
							<ColumnDependency style="margin-right: 0.5rem;" />
							<div class="truncate">References</div>
						</div>
					</Switch>
					<Switch>
						<div style="display: flex; align-items: left;">
							<ColumnDependency style="margin-right: 0.5rem;  transform: scaleX(-1);" />
							<div class="truncate">Reverse references</div>
						</div>
					</Switch>
				</ContentSwitcher>
				<div class="flex-auto overflow-auto">
					{#if referenceTab == 0}
						<div class="h-full" use:help={{ title: "References", description: "Other resources that this resource depends on, listed in the order stored in the game files." }}>
							<OrderedList class="h-full  overflow-y-auto" native>
								{#each dependencies as [hash, type, path, flag, inGame]}
									<ListItem class="p-1">
										{#if type}
											<ClickableTile
												on:click={async (e) => {
													trackEvent("Follow reference" + e.ctrlKey ? " in new tab " : " " + "from resource overview")

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
												<div class="break-all">{path || ""}</div>
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
									</ListItem>
								{/each}
							</OrderedList>
						</div>
					{/if}
					{#if referenceTab == 1}
						<div class="h-full" use:help={{ title: "Reverse references", description: "Other resources that depend upon this resource, sorted alphabetically." }}>
							<OrderedList class="h-full overflow-y-auto" native>
								{#each reverseDependencies as [hash, type, path]}
									<ListItem class="p-1">
										{#if type}
											<ClickableTile
												on:click={async (e) => {
													trackEvent("Follow reverse reference" + e.ctrlKey ? " in new tab " : " " + "from resource overview")

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
												<div class="break-all">{path || ""}</div>
											</ClickableTile>
										{:else}
											<div class="bg-[#303030] p-3">
												<div class="font-bold text-base -mt-1">{hash}</div>
												<div class="break-all">Unknown resource</div>
											</div>
										{/if}
									</ListItem>
								{/each}
							</OrderedList>
						</div>
					{/if}
				</div>
			</Pane>
		</Splitpanes>
	{:else}
		Loading...
	{/if}
</div>


<style>
	.no-scrollbar {
      -ms-overflow-style: none; /* IE and Edge */
      scrollbar-width: none; /* Firefox */
    }
</style>