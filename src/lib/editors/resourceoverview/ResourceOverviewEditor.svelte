<script lang="ts">
	import type { ResourceOverviewData, ResourceOverviewRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { Button } from "carbon-components-svelte"
	import { onMount } from "svelte"
	import Edit from "carbon-icons-svelte/lib/Edit.svelte"
	import Export from "carbon-icons-svelte/lib/Export.svelte"

	export let id: string

	let hash = ""
	let chunk = ""
	let pathOrHint: string | null = null
	let dependencies: [string, string, string | null, string][] = []
	let reverseDependencies: [string, string, string | null][] = []
	let data: ResourceOverviewData | null = null

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
				chunk = request.data.chunk_patch.replace(/patch[0-9]+/, "")
				pathOrHint = request.data.path_or_hint
				dependencies = request.data.dependencies
				reverseDependencies = request.data.reverse_dependencies
				data = request.data.data
				break

			// default:
			// 	request satisfies never
			// 	break
		}
	}
</script>

<div class="w-full h-full flex flex-col p-4">
	{#if data}
		{#if data.type === "Entity"}
			<div class="text-2xl mb-2 font-bold">
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
					icon={Export}
					on:click={async () => {
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
					icon={Export}
					on:click={async () => {
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
					icon={Export}
					on:click={async () => {
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
					icon={Export}
					on:click={async () => {
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
					icon={Export}
					on:click={async () => {
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
			<div class="grid grid-cols-2 gap-2 flex-grow basis-0">
				<div class="flex flex-col">
					<h4 class="mb-1">Dependencies</h4>
					<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
						{#each dependencies as [hash, type, path, flag]}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
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
							>
								<div class="text-base -mt-1"
									><span class="font-bold"
										>{hash}{#if type}.{type}{/if}</span
									>
									{flag}</div
								>
								<div class="break-all">{path || "No path"}</div>
							</div>
						{/each}
					</div>
				</div>
				<div class="flex flex-col">
					<h4 class="mb-1">Reverse dependencies</h4>
					<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
						{#each reverseDependencies as [hash, type, path]}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
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
							>
								<div class="font-bold text-base -mt-1"
									>{hash}{#if type}.{type}{/if}</div
								>
								<div class="break-all">{path || "No path"}</div>
							</div>
						{/each}
					</div>
				</div>
			</div>
		{:else if data.type === "Generic"}
			<div class="text-2xl mb-2 font-bold">
				{pathOrHint || "No path"}
			</div>
			<div class="flex flex-wrap gap-8 items-center mb-4">
				<div>
					<div>Hash</div>
					<div class="text-xl">{hash}</div>
				</div>
				<div>
					<div>Chunk</div>
					<div class="text-xl">{chunk}</div>
				</div>
			</div>
			<h4 class="mb-1">Actions</h4>
			<div class="flex flex-wrap gap-2 mb-4">
				<Button
					icon={Export}
					on:click={async () => {
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
			<div class="grid grid-cols-2 gap-2 flex-grow basis-0">
				<div class="flex flex-col">
					<h4 class="mb-1">Dependencies</h4>
					<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
						{#each dependencies as [hash, type, path, flag]}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
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
							>
								<div class="text-base -mt-1"
									><span class="font-bold"
										>{hash}{#if type}.{type}{/if}</span
									>
									{flag}</div
								>
								<div class="break-all">{path || "No path"}</div>
							</div>
						{/each}
					</div>
				</div>
				<div class="flex flex-col">
					<h4 class="mb-1">Reverse dependencies</h4>
					<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
						{#each reverseDependencies as [hash, type, path]}
							<div
								class="bg-[#303030] p-3 cursor-pointer"
								on:click={async () => {
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
							>
								<div class="font-bold text-base -mt-1"
									>{hash}{#if type}.{type}{/if}</div
								>
								<div class="break-all">{path || "No path"}</div>
							</div>
						{/each}
					</div>
				</div>
			</div>
		{/if}
	{:else}
		Loading...
	{/if}
</div>
