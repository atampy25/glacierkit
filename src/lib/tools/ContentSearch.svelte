<script lang="ts">
	import type { ContentSearchRequest } from "$lib/bindings-types"
	import { help } from "$lib/helpray"
	import { event } from "$lib/utils"
	import { trackEvent } from "@aptabase/tauri"
	import { Button, Checkbox, Search } from "carbon-components-svelte"
	import SearchIcon from "carbon-icons-svelte/lib/Search.svelte"
	import CheckboxIcon from "carbon-icons-svelte/lib/Checkbox.svelte"
	import CheckboxCheckedIcon from "carbon-icons-svelte/lib/CheckboxChecked.svelte"

	export async function handleRequest(request: ContentSearchRequest) {
		console.log("Content search tool handling request", request)

		switch (request.type) {
			case "setEnabled":
				enabled = request.data
				break

			case "setPartitions":
				allPartitions = request.data
				searchPartitions = Object.fromEntries(request.data.map((a) => [a, true]))
				break

			default:
				request satisfies never
				break
		}
	}

	let allPartitions: string[] = []

	let enabled = false
	let searchQuery = ""
	let searchEntities = false
	let searchRL = false
	let searchText = false
	let searchQN = false
	let searchLocalisation = false
	let searchPartitions: Record<string, boolean> = {}
</script>

<div
	class="w-full h-full p-2 overflow-y-auto"
	use:help={{ title: "Advanced search", description: "This panel lets you exhaustively search inside the contents of a variety of game formats with a given query." }}
>
	{#if !enabled}
		<div class="p-4">
			<p>You haven't selected a copy of the game to work with - go to the Settings tool to do that.</p>
		</div>
	{:else}
		<div class="pt-2 pb-1 px-2 text-base">
			<div class="mb-3">Search through the contents of files - not just their names.</div>
			<div class="mb-3"><Search placeholder="Search query (supports regex)" size="lg" bind:value={searchQuery} /></div>
			<div class="mb-4">
				<Checkbox labelText="Search entities" bind:checked={searchEntities} />
				<Checkbox labelText="Use QuickEntity format" bind:checked={searchQN} />
				<Checkbox labelText="Search ResourceLib types" bind:checked={searchRL} />
				<Checkbox labelText="Search textual files (JSON, REPO, ORES)" bind:checked={searchText} />
				<Checkbox labelText="Search localisation" bind:checked={searchLocalisation} />
			</div>
			<div class="mb-4">
				<div class="flex flex-wrap gap-2 items-center">
					<div>Partitions to search</div>
					<Button
						icon={CheckboxIcon}
						iconDescription="Deselect all partitions"
						size="small"
						on:click={async () => {
							searchPartitions = Object.fromEntries(allPartitions.map((a) => [a, false]))
						}}
					/>
					<Button
						icon={CheckboxCheckedIcon}
						iconDescription="Select all partitions"
						size="small"
						on:click={async () => {
							searchPartitions = Object.fromEntries(allPartitions.map((a) => [a, true]))
						}}
					/>
				</div>
				{#each allPartitions.entries() as [ind, partition]}
					<Checkbox labelText={`${partition} (chunk${ind})`} bind:checked={searchPartitions[partition]} />
				{/each}
			</div>
			<Button
				icon={SearchIcon}
				on:click={async () => {
					trackEvent("Perform content search", {
						searchEntities: String(searchEntities),
						searchRL: String(searchRL),
						searchText: String(searchText),
						partitions:
							Object.entries(searchPartitions).filter((a) => a[1]).length === Object.keys(searchPartitions).length
								? "all"
								: Object.entries(searchPartitions)
										.filter((a) => a[1])
										.map((a) => a[0])
										.join(", ")
					})

					const searchTypes = []

					if (searchEntities) searchTypes.push("TEMP")

					if (searchRL) searchTypes.push("AIRG", "RTLV", "ATMD", "VIDB", "UICB", "CPPT", "CBLU", "CRMD", "DSWB", "WSWB", "GFXF", "GIDX", "WSGB", "ECPB", "ENUM")

					if (searchText) searchTypes.push("JSON", "REPO", "ORES")

					if (searchLocalisation) searchTypes.push("CLNG", "DITL", "DLGE", "LOCR", "RTLV", "LINE")

					await event({
						type: "tool",
						data: {
							type: "contentSearch",
							data: {
								type: "search",
								data: [
									searchQuery,
									searchTypes,
									searchQN,
									Object.entries(searchPartitions)
										.filter((a) => a[1])
										.map((a) => a[0])
								]
							}
						}
					})
				}}>Start search</Button
			>
		</div>
	{/if}
</div>
