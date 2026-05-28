<script lang="ts">
	import type { ContentSearchResultsRequest } from "$lib/bindings"
	import { event } from "$lib/utils"
	import { onMount } from "svelte"
	import { trackEvent } from "$lib/utils"
	import { help } from "$lib/helpray"
	import { ClickableTile, Search } from "carbon-components-svelte"
	import { Filter } from "carbon-icons-svelte"
	import { VList } from "virtua/svelte"

	let { id }: { id: string } = $props()

	let query = $state("")
	let results: [string, string, string | null][] | null = $state(null)

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				editor: id,
				data: {
					type: "contentSearchResults",
					data: {
						type: "initialise"
					}
				}
			}
		})
	})

	export async function handleRequest(request: ContentSearchResultsRequest) {
		console.log(`Content search results page ${id} handling request`, request)

		switch (request.type) {
			case "initialise":
				query = request.data.query
				results = request.data.results
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}

	let search = $state("")
</script>

<div
	class="w-full h-full flex flex-col p-4 overflow-y-auto"
	use:help={{ title: "Search results", description: "This page lists all the resources matching a previous search made from the Advanced Search panel." }}
>
	{#if results}
		{#if results.length}
			<h4 class="mb-2">Results for <code>{query}</code></h4>
			<div class="mb-2 pr-2">
				<Search placeholder="Filter..." icon={Filter} size="lg" bind:value={search} />
			</div>
			<div class="flex-grow basis-0 overflow-y-auto pr-2">
				<VList data={results.filter((a) => `${a[0]}.${a[1]}|${a[2]}`.toLowerCase().includes(search.toLowerCase()))} getKey={(a) => a[0]}>
					{#snippet children([hash, type, path])}
						<ClickableTile
							style="min-height: unset"
							class="mb-2"
							onclick={async () => {
								trackEvent("Open result from content search results page")

								await event({
									type: "editor",
									data: {
										editor: id,
										data: {
											type: "contentSearchResults",
											data: {
												type: "openResourceOverview",
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
		{:else}
			No results
		{/if}
	{:else}
		Loading...
	{/if}
</div>
