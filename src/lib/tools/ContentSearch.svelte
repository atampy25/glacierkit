<script lang="ts">
	import type { ContentSearchRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { trackEvent } from "@aptabase/tauri"
	import { Button, Checkbox, Search } from "carbon-components-svelte"
	import SearchIcon from "carbon-icons-svelte/lib/Search.svelte"

	export async function handleRequest(request: ContentSearchRequest) {
		console.log("Content search tool handling request", request)

		switch (request.type) {
			case "setEnabled":
				enabled = request.data
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}

	let enabled = false
	let searchQuery = ""
	let searchEntities = false
	let searchRL = false
	let searchText = false
	let searchQN = false
</script>

<div class="w-full h-full p-2">
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
				<Checkbox labelText="Search ResourceLib types" bind:checked={searchRL} />
				<Checkbox labelText="Search textual files (JSON, REPO, ORES)" bind:checked={searchText} />
				<Checkbox labelText="Use QuickEntity format" bind:checked={searchQN} />
			</div>
			<Button
				icon={SearchIcon}
				on:click={async () => {
					trackEvent("Perform content search", { searchEntities: String(searchEntities), searchRL: String(searchRL), searchText: String(searchText) })

					const searchTypes = []

					if (searchEntities) searchTypes.push("TEMP")

					// Support for CBLUs in RT is broken so they are not included in search
					if (searchRL) searchTypes.push("AIRG", "RTLV", "ATMD", "VIDB", "UICB", "CPPT", "CRMD", "DSWB", "WSWB", "GFXF", "GIDX", "WSGB", "ECPB", "ENUM")

					if (searchText) searchTypes.push("JSON", "REPO", "ORES")

					await event({
						type: "tool",
						data: {
							type: "contentSearch",
							data: {
								type: "search",
								data: [searchQuery, searchTypes, searchQN]
							}
						}
					})
				}}>Start search</Button
			>
		</div>
	{/if}
</div>
