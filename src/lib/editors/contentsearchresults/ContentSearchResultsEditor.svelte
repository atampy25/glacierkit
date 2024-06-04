<script lang="ts">
	import type { ContentSearchResultsRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { onMount } from "svelte"
	import { trackEvent } from "@aptabase/tauri"
	import { help } from "$lib/helpray"

	export let id: string

	let results: [string, string, string | null][] = []

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				type: "contentSearchResults",
				data: {
					type: "initialise",
					data: {
						id
					}
				}
			}
		})
	})

	export async function handleRequest(request: ContentSearchResultsRequest) {
		console.log(`Content search results page ${id} handling request`, request)

		switch (request.type) {
			case "initialise":
				results = request.data.results
				break

			// No exhaustivity check, only one request type
			// default:
			// 	request satisfies never
			// 	break
		}
	}
</script>

<div
	class="w-full h-full flex flex-col p-4 overflow-y-auto"
	use:help={{ title: "Search results", description: "This page lists all the resources matching a previous search made from the Advanced Search panel." }}
>
	{#if results.length}
		<h4 class="mb-1">Results</h4>
		<div class="flex-grow basis-0 overflow-y-auto flex flex-col gap-1 pr-2">
			{#each results as [hash, type, path]}
				<div
					class="bg-[#303030] p-3 cursor-pointer"
					on:click={async () => {
						trackEvent("Open result from content search results page")

						await event({
							type: "editor",
							data: {
								type: "contentSearchResults",
								data: {
									type: "openResourceOverview",
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
			{/each}
		</div>
	{:else}
		No results
	{/if}
</div>
