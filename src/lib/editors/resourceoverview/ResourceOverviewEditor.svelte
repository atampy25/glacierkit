<script lang="ts">
	import type { ReferenceFlags, ResourceChangelogEntry, ResourceOverviewData, ResourceOverviewRequest } from "$lib/bindings"
	import { event } from "$lib/utils"
	import { Button, ClickableTile, DataTable, Search, Tab, Table, TableBody, TableCell, TableHead, TableHeader, TableRow, Tabs, Tile } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Edit from "carbon-icons-svelte/lib/Edit.svelte"
	import DocumentExport from "carbon-icons-svelte/lib/DocumentExport.svelte"
	import { trackEvent } from "$lib/utils"
	import WaveformPlayer from "$lib/components/WaveformPlayer.svelte"
	import MultiWaveformPlayer from "$lib/components/MultiWaveformPlayer.svelte"
	import Monaco from "./Monaco.svelte"
	import { v4 } from "uuid"
	import { help } from "$lib/helpray"
	import MeshPreview from "$lib/components/MeshPreview.svelte"
	import { Pane, Splitpanes } from "svelte-splitpanes"
	import { ColumnDependency, Filter, SoftwareResource, TrashCan } from "carbon-icons-svelte"
	import AddLarge from "carbon-icons-svelte/lib/AddLarge.svelte"
	import { VList } from "virtua/svelte"
	import { platform } from "@tauri-apps/plugin-os"

	let { id }: { id: string } = $props()

	let hash = $state("")
	let filetype = $state("")
	let partition = $state("")
	let pathOrHint: string | null = $state(null)
	let dependencies: [string, string, string | null, ReferenceFlags, boolean][] = $state([])
	let reverseDependencies: [string, string, string | null][] = $state([])
	let changelog: ResourceChangelogEntry[] = $state([])
	let data: ResourceOverviewData | null = $state(null)

	let previewImage: HTMLImageElement | null = $state(null)
	let referenceTab = $state(0)

	const typesWithPreview = ["Image", "Mesh", "Audio", "MultiAudio", "GenericRL", "Json", "HMLanguages", "LocalisedLine", "MaterialInstance", "MaterialEntity", "SoundDefinitions"]

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				editor: id,
				data: {
					type: "resourceOverview",
					data: {
						type: "initialise"
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
				partition = request.data.chunkPatch.replace(/patch[0-9]+/, "")
				pathOrHint = request.data.pathOrHint
				dependencies = request.data.dependencies
				reverseDependencies = request.data.reverseDependencies
				changelog = request.data.changelog
				data = request.data.data
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}

	let refSearch = $state("")
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
											class="h-[30vh] bg-[#7f7f7f]"
											style="image-rendering: pixelated"
											bind:this={previewImage}
											on:load={() => {
												previewImage = previewImage
											}}
											src="{platform() === 'windows' ? 'https://editor-asset.localhost' : 'editor-asset:/'}/{id}/{data.data.asset_id}"
											alt="Resource preview"
										/>
									{:else if data.type === "Mesh"}
										<div class="h-[30vh]">
											<MeshPreview
												src="{platform() === 'windows' ? 'https://editor-asset.localhost' : 'editor-asset:/'}/{id}/{data.data.asset_id}"
												boundingBox={data.data.bounding_box}
											/>
										</div>
									{:else if data.type === "Audio"}
										{#if data.data.asset_id}
											<WaveformPlayer src="{platform() === 'windows' ? 'https://editor-asset.localhost' : 'editor-asset:/'}/{id}/{data.data.asset_id}" />
										{:else}
											<div class="text-lg">This audio object is in an unsupported format (likely MIDI).</div>
										{/if}
									{:else if data.type === "MultiAudio"}
										<div class="text-neutral-400 mb-2">{data.data.name}</div>
										{#if data.data.audios.length}
											<MultiWaveformPlayer
												src={data.data.audios.map((a) => [
													a[0],
													a[1] ? `${platform() === "windows" ? "https://editor-asset.localhost" : "editor-asset:/"}/${id}/${a[1]}` : null
												])}
												on:download={async ({ detail }) => {
													trackEvent("Extract specific audio from WWEV file")

													await event({
														type: "editor",
														data: {
															editor: id,
															data: {
																type: "resourceOverview",
																data: {
																	type: "extractSpecificMultiOgg",
																	data: {
																		index: detail
																	}
																}
															}
														}
													})
												}}
											/>
										{:else}
											<div class="-mt-1 text-lg">No linked audio</div>
										{/if}
									{:else if data.type === "GenericRL" || data.type === "Json" || data.type === "HMLanguages" || data.type === "MaterialInstance" || data.type === "MaterialEntity" || data.type === "SoundDefinitions"}
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "openInEditor"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsQN"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
															}
														}
													}
												})
											}}>Extract TEMP as binary file</Button
										>
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract TEMP as JSON")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractTEMPAsRT"
															}
														}
													}
												})
											}}>Extract TEMP as JSON</Button
										>
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract TBLU as binary file")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractTBLUAsFile"
															}
														}
													}
												})
											}}>Extract TBLU as binary file</Button
										>
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract TBLU as JSON")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractTBLUAsRT"
															}
														}
													}
												})
											}}>Extract TBLU as JSON</Button
										>
									{:else if data.type === "Image"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												// Analytics tracked on Rust end

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsImage"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
															}
														}
													}
												})
											}}>Extract file</Button
										>
									{:else if data.type === "Audio"}
										{#if data.data.asset_id}
											<Button
												icon={DocumentExport}
												on:click={async () => {
													trackEvent("Extract audio file as OGG")

													await event({
														type: "editor",
														data: {
															editor: id,
															data: {
																type: "resourceOverview",
																data: {
																	type: "extractAsOgg"
																}
															}
														}
													})
												}}>Extract as OGG</Button
											>
										{/if}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract audio file as original")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
												trackEvent("Extract WWEV file as OGGs")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractMultiOgg"
															}
														}
													}
												})
											}}>Extract all as OGGs</Button
										>
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract WWEV file as original")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
												trackEvent("Extract generic BIN1 file as JSON", { hash, filetype })

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsRTGeneric"
															}
														}
													}
												})
											}}>Extract as JSON</Button
										>
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract generic BIN1 file as binary", { hash, filetype })

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "openInEditor"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "openInEditor"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsRTGeneric"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsHMLanguages"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
															}
														}
													}
												})
											}}>Extract file</Button
										>
									{:else if data.type === "MaterialInstance"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract material instance file as original")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
															}
														}
													}
												})
											}}>Extract file</Button
										>
									{:else if data.type === "MaterialEntity"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract material entity file as original")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
															}
														}
													}
												})
											}}>Extract file</Button
										>
									{:else if data.type === "SoundDefinitions"}
										<Button
											icon={DocumentExport}
											on:click={async () => {
												trackEvent("Extract sound definitions file as original")

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
														editor: id,
														data: {
															type: "resourceOverview",
															data: {
																type: "extractAsFile"
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
					<Tabs autoWidth class="mb-2" bind:selected={referenceTab}>
						<Tab>
							<div class="flex items-center gap-2">
								<ColumnDependency class="flex-shrink-0" />
								<div>References</div>
							</div>
						</Tab>
						<Tab>
							<div class="flex items-center gap-2">
								<ColumnDependency class="flex-shrink-0 -scale-x-100" />
								<div>Reverse references</div>
							</div>
						</Tab>
					</Tabs>
					<div class="mb-2 pr-2">
						<Search placeholder="Filter..." icon={Filter} size="lg" bind:value={refSearch} />
					</div>
					{#if referenceTab == 0}
						<div
							class="h-full overflow-y-auto pr-2"
							use:help={{ title: "References", description: "Other resources that this resource depends on, listed in the order stored in the game files." }}
						>
							<VList data={dependencies.filter((a) => `${a[0]}.${a[1]}|${a[2]}`.toLowerCase().includes(refSearch.toLowerCase()))} getKey={(a) => a[0]}>
								{#snippet children([hash, type, path, flags, inGame])}
									{#if inGame}
										<ClickableTile
											style="min-height: unset"
											class="mb-2"
											on:click={async (e) => {
												trackEvent(`Follow reference ${e.ctrlKey ? "in new tab " : "from resource overview"}`)

												await event({
													type: "editor",
													data: {
														editor: id,
														data: {
															type: "resourceOverview",
															data: !e.ctrlKey
																? {
																		type: "followDependency",
																		data: {
																			newHash: hash
																		}
																	}
																: {
																		type: "followDependencyInNewTab",
																		data: {
																			hash
																		}
																	}
														}
													}
												})
											}}
										>
											<div class="text-base -mt-1"
												><span class="font-bold">{hash}.{type}</span>
												{flags.type ? flags.type[0].toUpperCase() + flags.type.slice(1) : "Install"}{flags.acquired ? ", acquired" : ""}{flags.languageCode
													? `, language ${flags.languageCode}`
													: ""}</div
											>
											<div class="break-all">{path || "No path"}</div>
										</ClickableTile>
									{:else}
										<div class="bg-[#303030] p-3 mb-2">
											<div class="text-base -mt-1"
												><span class="font-bold">{hash}</span>
												{flags.type ? flags.type[0].toUpperCase() + flags.type.slice(1) : "Install"}{flags.acquired ? ", acquired" : ""}{flags.languageCode
													? `, language ${flags.languageCode}`
													: ""}</div
											>
											<div class="break-all">Unknown resource</div>
											{#if !inGame}
												<div class="text-base">Not present in game files</div>
											{/if}
										</div>
									{/if}
								{/snippet}
							</VList>
						</div>
					{/if}
					{#if referenceTab == 1}
						<div class="h-full overflow-y-auto pr-2" use:help={{ title: "Reverse references", description: "Other resources that depend upon this resource, sorted alphabetically." }}>
							<VList data={reverseDependencies.filter((a) => `${a[0]}.${a[1]}|${a[2]}`.toLowerCase().includes(refSearch.toLowerCase()))}>
								{#snippet children([hash, type, path])}
									<ClickableTile
										style="min-height: unset"
										class="mb-2"
										on:click={async (e) => {
											trackEvent(`Follow reverse reference ${e.ctrlKey ? "in new tab " : "from resource overview"}`)

											await event({
												type: "editor",
												data: {
													editor: id,
													data: {
														type: "resourceOverview",
														data: !e.ctrlKey
															? {
																	type: "followDependency",
																	data: {
																		newHash: hash
																	}
																}
															: {
																	type: "followDependencyInNewTab",
																	data: {
																		hash
																	}
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
								{/snippet}
							</VList>
						</div>
					{/if}
				</Pane>
			</Splitpanes>
		</div>
	{:else}
		Loading...
	{/if}
</div>
