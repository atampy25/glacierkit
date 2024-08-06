<script lang="ts">
	import type { RepositoryItemInformation, RepositoryPatchEditorRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"
	import { onMount } from "svelte"
	import Monaco from "./Monaco.svelte"
	import { debounce } from "lodash"
	import { Button, Dropdown, Search } from "carbon-components-svelte"
	import Undo from "carbon-icons-svelte/lib/Undo.svelte"
	import Add from "carbon-icons-svelte/lib/Add.svelte"
	import Filter from "carbon-icons-svelte/lib/Filter.svelte"
	import { help } from "$lib/helpray"

	export let id: string

	let monacoEditor: Monaco

	let selectedItem: string | null = null

	let repositoryItems: [string, RepositoryItemInformation][] = []

	let modifiedRepositoryItems: Set<string> = new Set()

	let searchQuery = ""
	let searchFilter: RepositoryItemInformation["type"] | "All" = "All"

	const debouncedUpdateFunction = { run: debounce(async (_: string) => {}, 500) }

	export async function handleRequest(request: RepositoryPatchEditorRequest) {
		console.log(`Repository patch editor ${id} handling request`, request)

		switch (request.type) {
			case "setRepositoryItems":
				repositoryItems = request.data.items
				break

			case "setModifiedRepositoryItems":
				modifiedRepositoryItems = new Set(request.data.modified)
				break

			case "addNewRepositoryItem":
				repositoryItems.push(request.data.new_item)
				break

			case "removeRepositoryItem":
				repositoryItems = repositoryItems.filter((a) => a[0] !== request.data.item)
				break

			case "setMonacoContent":
				selectedItem = request.data.item

				debouncedUpdateFunction.run = debounce((d) => {
					contentChanged(request.data.item, d)
				}, 500)

				setTimeout(() => monacoEditor.setContents(request.data.orig_data, request.data.data), 0)
				break

			case "deselectMonaco":
				selectedItem = null
				break

			case "modifyItemInformation":
				repositoryItems.find((a) => a[0] === request.data.item)![1] = request.data.info
				repositoryItems = repositoryItems
				break

			default:
				request satisfies never
				break
		}
	}

	async function contentChanged(item: string, content: string) {
		try {
			JSON.parse(content)
		} catch {
			return
		}

		await event({
			type: "editor",
			data: {
				type: "repositoryPatch",
				data: {
					type: "modifyItem",
					data: {
						id,
						item: item,
						data: content
					}
				}
			}
		})
	}

	onMount(async () => {
		await event({
			type: "editor",
			data: {
				type: "repositoryPatch",
				data: {
					type: "initialise",
					data: {
						id
					}
				}
			}
		})
	})

	function searchInput(evt: any) {
		const _event = evt as { target: HTMLInputElement }

		searchQuery = _event.target.value.toLowerCase()
	}
</script>

<div class="grid grid-cols-4 gap-4 w-full h-full">
	<div class="h-full">
		<div class="h-1/3 flex flex-col" use:help={{ title: "Modified repository items", description: "Items that have been added or modified from their original state by your edits." }}>
			<h2>Modified</h2>
			<div class="mt-1">
				<Button
					icon={Add}
					on:click={async () => {
						await event({
							type: "editor",
							data: {
								type: "repositoryPatch",
								data: {
									type: "createRepositoryItem",
									data: {
										id
									}
								}
							}
						})
					}}
				>
					New item
				</Button>
			</div>
			<div class="mt-2 basis-0 flex-grow flex flex-col gap-1 overflow-y-auto">
				{#each repositoryItems.filter((a) => modifiedRepositoryItems.has(a[0])) as [itemId, info] (itemId)}
					<div
						class="p-4 bg-neutral-900 flex items-center cursor-pointer break-all mr-2"
						on:click={async () => {
							await event({
								type: "editor",
								data: {
									type: "repositoryPatch",
									data: {
										type: "selectItem",
										data: { id, item: itemId }
									}
								}
							})
						}}
					>
						{#if info.type === "NPC"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-user" /> NPC
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Item"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-wrench" /> Item
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Weapon"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Weapon
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Outfit"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shirt" /> Outfit
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AmmoBehaviour"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Ammo Behaviour
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AmmoConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Ammo Config
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MagazineConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Magazine Config
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.size} / {info.data.tags.join(", ")}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "DifficultyParameter"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gear" /> Difficulty Parameter
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MapArea"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-location-dot" /> Map Area
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MasteryItem"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Mastery Item
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Modifier"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-cog" /> Modifier
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.kind}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Setpiece"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Setpiece
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.traits.join(", ")}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "WeaponConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Weapon Config
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{:else if info.type === "ScoreMultiplier"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-chart-simple" /> Score Multiplier
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "ItemBundle"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Item Bundle
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "ItemList"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-list" /> Item List
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{:else}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-circle-question" /> Unknown
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		</div>
		<div class="h-2/3 flex flex-col" use:help={{ title: "Unmodified repository items", description: "Repository items that have not been touched by your edits." }}>
			<h2>Unmodified</h2>
			<div class="mt-1 flex gap-2">
				<Search
					placeholder="Filter..."
					icon={Filter}
					size="lg"
					on:change={searchInput}
					on:clear={() => {
						searchQuery = ""
					}}
				/>
				<Dropdown
					class="w-60 no-menu-spacing"
					bind:selectedId={searchFilter}
					items={[
						{ id: "All", text: "All" },
						{ id: "NPC", text: "NPCs" },
						{ id: "Item", text: "Items" },
						{ id: "Weapon", text: "Weapons" },
						{ id: "Outfit", text: "Outfits" },
						{ id: "AmmoBehaviour", text: "Ammo Behaviours" },
						{ id: "AmmoConfig", text: "Ammo Configs" },
						{ id: "MagazineConfig", text: "Magazine Configs" },
						{ id: "DifficultyParameter", text: "Difficulty Params" },
						{ id: "MapArea", text: "Map Areas" },
						{ id: "MasteryItem", text: "Mastery Items" },
						{ id: "Modifier", text: "Modifiers" },
						{ id: "Setpiece", text: "Setpieces" },
						{ id: "WeaponConfig", text: "Weapon Configs" },
						{ id: "ScoreMultiplier", text: "Score Multipliers" },
						{ id: "ItemBundle", text: "Item Bundles" },
						{ id: "ItemList", text: "Item Lists" },
						{ id: "Unknown", text: "Uncategorised" }
					]}
					on:select={({ detail: { selectedId } }) => {
						searchFilter = selectedId
					}}
				/>
			</div>
			<div class="mt-2 basis-0 flex-grow flex flex-col gap-1 overflow-y-auto">
				{#each repositoryItems
					.filter((a) => searchFilter === "All" || a[1].type === searchFilter)
					.filter((a) => (searchQuery ? searchQuery.split(" ").every((b) => JSON.stringify(a).toLowerCase().includes(b)) : true))
					.filter((a) => !modifiedRepositoryItems.has(a[0])) as [itemId, info] (itemId)}
					<div
						class="p-4 bg-neutral-900 flex items-center cursor-pointer break-all mr-2"
						on:click={async () => {
							await event({
								type: "editor",
								data: {
									type: "repositoryPatch",
									data: {
										type: "selectItem",
										data: { id, item: itemId }
									}
								}
							})
						}}
					>
						{#if info.type === "NPC"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-user" /> NPC
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Item"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-wrench" /> Item
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Weapon"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Weapon
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Outfit"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shirt" /> Outfit
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AmmoBehaviour"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Ammo Behaviour
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AmmoConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Ammo Config
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MagazineConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Magazine Config
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.size} / {info.data.tags.join(", ")}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "DifficultyParameter"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gear" /> Difficulty Param
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MapArea"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-location-dot" /> Map Area
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "MasteryItem"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Mastery Item
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Modifier"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-cog" /> Modifier
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.kind}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Setpiece"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Setpiece
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.traits.join(", ")}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "WeaponConfig"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-gun" /> Weapon Config
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{:else if info.type === "ScoreMultiplier"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-chart-simple" /> Score Multiplier
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "ItemBundle"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Item Bundle
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.name}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "ItemList"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-list" /> Item List
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{:else}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-circle-question" /> Unknown
							</div>
							<div class="text-neutral-300">
								{itemId}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		</div>
	</div>
	<div class="col-span-3">
		{#if selectedItem}
			<div
				class="mb-2 flex items-center"
				use:help={{ title: "Editor", description: "You can see and edit the data of the selected repository item here. On the right is your modified version; on the left is the original." }}
			>
				<div class="flex-grow">
					<h2>Editor</h2>
					<code>{selectedItem}</code>
				</div>
				<Button
					kind="danger"
					icon={Undo}
					on:click={async () => {
						if (selectedItem) {
							await event({
								type: "editor",
								data: {
									type: "repositoryPatch",
									data: {
										type: "resetModifications",
										data: {
											id,
											item: selectedItem
										}
									}
								}
							})
						}
					}}
				>
					Reset changes
				</Button>
			</div>
			<div class="overflow-visible" style="height: calc(100vh - 11rem - 1.5rem)">
				{#key selectedItem}
					<Monaco {id} on:contentChanged={({ detail }) => debouncedUpdateFunction.run(detail)} bind:this={monacoEditor} />
				{/key}
			</div>
		{:else}
			<div class="w-full h-full flex items-center justify-center text-xl">Select a repository item on the left to edit it here.</div>
		{/if}
	</div>
</div>
