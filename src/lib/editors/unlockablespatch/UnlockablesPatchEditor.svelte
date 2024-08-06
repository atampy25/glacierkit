<script lang="ts">
	import type { UnlockableInformation, UnlockablesPatchEditorRequest } from "$lib/bindings-types"
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

	let unlockables: [string, UnlockableInformation][] = []

	let modifiedUnlockables: Set<string> = new Set()

	let searchQuery = ""
	let searchFilter: UnlockableInformation["type"] | "All" = "All"

	const debouncedUpdateFunction = { run: debounce(async (_: string) => {}, 500) }

	export async function handleRequest(request: UnlockablesPatchEditorRequest) {
		console.log(`Unlockables patch editor ${id} handling request`, request)

		switch (request.type) {
			case "setUnlockables":
				unlockables = request.data.unlockables
				break

			case "setModifiedUnlockables":
				modifiedUnlockables = new Set(request.data.modified)
				break

			case "addNewUnlockable":
				unlockables.push(request.data.new_unlockable)
				break

			case "removeUnlockable":
				unlockables = unlockables.filter((a) => a[0] !== request.data.unlockable)
				break

			case "setMonacoContent":
				selectedItem = request.data.unlockable

				debouncedUpdateFunction.run = debounce((d) => {
					contentChanged(request.data.unlockable, d)
				}, 500)

				if (JSON.parse(request.data.data).Id) {
					hasIdAttribute = true
				} else {
					hasIdAttribute = false
				}

				setTimeout(() => monacoEditor.setContents(request.data.orig_data, request.data.data), 0)
				break

			case "deselectMonaco":
				selectedItem = null
				break

			case "modifyUnlockableInformation":
				unlockables.find((a) => a[0] === request.data.unlockable)![1] = request.data.info
				unlockables = unlockables
				break

			default:
				request satisfies never
				break
		}
	}

	let hasIdAttribute = true

	async function contentChanged(item: string, content: string) {
		try {
			if (JSON.parse(content).Id) {
				hasIdAttribute = true
			} else {
				hasIdAttribute = false
			}
		} catch {
			return
		}

		await event({
			type: "editor",
			data: {
				type: "unlockablesPatch",
				data: {
					type: "modifyUnlockable",
					data: {
						id,
						unlockable: item,
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
				type: "unlockablesPatch",
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
		<div class="h-1/3 flex flex-col" use:help={{ title: "Modified unlockables", description: "Unlockables that have been added or modified from their original state by your edits." }}>
			<h2>Modified</h2>
			<div class="mt-1">
				<Button
					icon={Add}
					on:click={async () => {
						await event({
							type: "editor",
							data: {
								type: "unlockablesPatch",
								data: {
									type: "createUnlockable",
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
				{#each unlockables.filter((a) => modifiedUnlockables.has(a[0])) as [itemId, info] (itemId)}
					<div
						class="p-4 bg-neutral-900 flex items-center cursor-pointer break-all mr-2"
						on:click={async () => {
							await event({
								type: "editor",
								data: {
									type: "unlockablesPatch",
									data: {
										type: "selectUnlockable",
										data: { id, unlockable: itemId }
									}
								}
							})
						}}
					>
						{#if info.type === "Access"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Access
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "EvergreenMastery"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Evergreen Mastery
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AgencyPickup"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-box-open" /> Agency Pickup
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Disguise"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shirt" /> Disguise
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Gear"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-wrench" /> Gear
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
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
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Location"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-location-dot" /> Location
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "LoadoutUnlock"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Loadout Unlock
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Package"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-box" /> Package
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-circle-question" /> Unknown
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{/if}
					</div>
				{/each}
			</div>
		</div>
		<div class="h-2/3 flex flex-col" use:help={{ title: "Unmodified unlockables", description: "Unlockables that have not been touched by your edits." }}>
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
						{ id: "Access", text: "Accesses" },
						{ id: "EvergreenMastery", text: "Evergreen Masteries" },
						{ id: "AgencyPickup", text: "Agency Pickups" },
						{ id: "Disguise", text: "Disguises" },
						{ id: "Gear", text: "Gears" },
						{ id: "Weapon", text: "Weapons" },
						{ id: "Location", text: "Locations" },
						{ id: "LoadoutUnlock", text: "Loadout Unlocks" },
						{ id: "Package", text: "Packages" },
						{ id: "Unknown", text: "Uncategorised" }
					]}
					on:select={({ detail: { selectedId } }) => {
						searchFilter = selectedId
					}}
				/>
			</div>
			<div class="mt-2 basis-0 flex-grow flex flex-col gap-1 overflow-y-auto">
				{#each unlockables
					.filter((a) => searchFilter === "All" || a[1].type === searchFilter)
					.filter((a) => (searchQuery ? searchQuery.split(" ").every((b) => JSON.stringify(a).toLowerCase().includes(b)) : true))
					.filter((a) => !modifiedUnlockables.has(a[0])) as [itemId, info] (itemId)}
					<div
						class="p-4 bg-neutral-900 flex items-center cursor-pointer break-all mr-2"
						on:click={async () => {
							await event({
								type: "editor",
								data: {
									type: "unlockablesPatch",
									data: {
										type: "selectUnlockable",
										data: { id, unlockable: itemId }
									}
								}
							})
						}}
					>
						{#if info.type === "Access"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Access
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "EvergreenMastery"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-star" /> Evergreen Mastery
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "AgencyPickup"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-box-open" /> Agency Pickup
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Disguise"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shirt" /> Disguise
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Gear"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-wrench" /> Gear
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
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
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Location"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-location-dot" /> Location
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "LoadoutUnlock"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-shapes" /> Loadout Unlock
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else if info.type === "Package"}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-solid fa-box" /> Package
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
							</div>
						{:else}
							<div class="my-1 flex gap-2 flex-shrink-0 w-[30%] mr-2">
								<i class="fa-regular fa-circle-question" /> Unknown
							</div>
							<div>
								<div class="text-lg font-bold">
									{info.data.id || "No ID"}
								</div>
								<div class="text-neutral-300">
									{itemId}
								</div>
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
				use:help={{ title: "Editor", description: "You can see and edit the data of the selected unlockable here. On the right is your modified version; on the left is the original." }}
			>
				<div class="flex-grow">
					<h2>Editor</h2>
					<div class="flex gap-4 items-center">
						<code>{selectedItem}</code>
						{#if !hasIdAttribute}
							<span class="text-red-200">Must have Id attribute</span>
						{/if}
					</div>
				</div>
				<Button
					kind="danger"
					icon={Undo}
					on:click={async () => {
						if (selectedItem) {
							await event({
								type: "editor",
								data: {
									type: "unlockablesPatch",
									data: {
										type: "resetModifications",
										data: {
											id,
											unlockable: selectedItem
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
