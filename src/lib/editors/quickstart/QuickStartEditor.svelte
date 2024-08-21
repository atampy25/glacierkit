<script lang="ts">
	import type { QuickStartRequest, ProjectInfo } from "$lib/bindings-types"
	import { event } from "$lib/utils"

	import { onMount } from "svelte"

	import { help } from "$lib/helpray"
	import { Button, ClickableTile, Link, OutboundLink, OverflowMenu, OverflowMenuItem, Tile } from "carbon-components-svelte"
	import { FolderAdd, FolderOpen, LogoDiscord, LogoGithub, Time, WorshipMuslim } from "carbon-icons-svelte"
	import { Pane, Splitpanes } from "svelte-splitpanes"
	import { dialog } from "@tauri-apps/api"
	import { shell } from "@tauri-apps/api"

	export let id: string

	let recent_projects: ProjectInfo[] = []

	export async function handleRequest(request: QuickStartRequest) {
		console.log(`Start menu ${id} handling request`, request)

		switch (request.type) {
			case "initialise":
				recent_projects = request.data.recent_projects
				break

			default:
				request satisfies never
				break
		}
	}
</script>

<div
	class="w-full h-full flex flex-col p-4 overflow-y-auto"
	use:help={{
		title: "Project hub",
		description: "The Project hub helps you with the first steps when you launch Glacierkit."
	}}
>
	<div class="p-8">
		<h1 class="text-2xlg">Welcome to GlacierKit</h1>
		<p>An integrated modding tool for the HITMAN World of Assassination trilogy.</p>
	</div>
	<div class="flex">
		<div class="flex-none pr-16">
			<div class="flex flex-col justify-between">
				<div>
					<h4 class="mb-1 text-base font-semibold">New Project</h4>
				</div>
				<div class="flex flex-col">
					<Link
						class="flex items-center py-1"
						on:click={async () => {
							const selected = await dialog.open({
								directory: true,
								multiple: false
							})
							if (typeof selected === "string") {
                                //TODO: add call to event to initialize a project
								await event({ type: "global", data: { type: "loadWorkspace", data: selected } })
								await event({ type: "editor", data: { type: "quickStart", data: { type: "addRecentProject", data: { path: selected } } } })
							}
						}}
					>
						<FolderAdd title="Local" class="h-5 w-5 mr-2" />
						<span class="text-sm">create locally</span>
					</Link>

					<OutboundLink
						class="flex items-center py-1"
						on:click={async () => {
							await shell.open("https://github.com/new?template_name=smf-mod&template_owner=atampy25")
						}}
					>
						<LogoGithub title="Local" class="h-5 w-5 mr-2" />
						<span class="text-sm">create with source control</span>
					</OutboundLink>
				</div>
			</div>

			<div class="flex flex-col justify-between pt-8">
				<div>
					<h4 class="mb-1 text-base font-semibold">Open Project</h4>
				</div>
				<div class="flex flex-col">
					<Link
						class="flex items-center py-1"
						on:click={async () => {
							const selected = await dialog.open({
								directory: true,
								multiple: false
							})
							if (typeof selected === "string") {
								await event({ type: "global", data: { type: "loadWorkspace", data: selected } })
                                await event({ type: "editor", data: { type: "quickStart", data: { type: "addRecentProject", data: { path: selected } } } })
							}
						}}
					>
						<FolderOpen title="Local" class="h-5 w-5 mr-2" />
						<span class="text-sm">open from disk</span>
					</Link>
				</div>
			</div>

			<div class="flex flex-col justify-between pt-16">
				<div>
					<h4 class="mb-1 text-base font-semibold">Documentation</h4>
				</div>
				<div class="flex flex-col">
					<OutboundLink
						class="flex items-center py-1"
						on:click={async () => {
							await shell.open("https://discord.gg/wBwDH3W6")
						}}
					>
						<LogoDiscord title="Discord" class="h-5 w-5 mr-2" />
						<span class="text-sm">discord server</span>
					</OutboundLink>
				</div>
				<div class="flex flex-col">
					<OutboundLink
						class="flex items-center py-1"
						on:click={async () => {
							await shell.open("https://github.com/glacier-modding")
						}}
					>
						<LogoGithub title="Github" class="h-5 w-5 mr-2" />
						<span class="text-sm">github org</span>
					</OutboundLink>
				</div>
				<div class="flex flex-col">
					<OutboundLink
						class="flex items-center py-1"
						on:click={async () => {
							await shell.open("https://glaciermodding.org/")
						}}
					>
						<WorshipMuslim title="Wiki" class="h-5 w-5 mr-2" />
						<span class="text-sm">wiki</span>
					</OutboundLink>
				</div>
			</div>
		</div>
		<div class="flex-1">
			<div class="flex flex-col justify-between">
				<div>
					<h4 class="mb-1 text-base font-semibold">Recent Projects</h4>
				</div>
				<div class="flex flex-col">
					{#each recent_projects as project}
						<ClickableTile
							class="mt-2"
							on:click={async () => {
								await event({ type: "global", data: { type: "loadWorkspace", data: project.path } })
								await event({ type: "global", data: { type: "removeTab", data: id } }) //sudoku
							}}
						>
							<div class="flex flex-col justify-between h-full">
								<div>
									<div class="flex">
										<h2 class="text-lg font-semibold flex-grow">{project.name}</h2>
										<p class="text-sm">({project.version})</p>
									</div>
									<div class="flex items-center justify-center">
										<div class="flex-grow">
											<p class="pt-1 text-xs text-slate-400">{project.path}</p>
										</div>
										<OverflowMenu
											flipped
											on:click={(e) => {
												e.stopPropagation()
											}}
										>
											<OverflowMenuItem
												text="Show in explorer"
												on:click={(e) => {
													e.stopPropagation()
													shell.open(project.path)
												}}
											/>
											<OverflowMenuItem
												danger
												text="Remove recent"
												on:click={async (e) => {
													e.stopPropagation()
													await event({ type: "editor", data: { type: "quickStart", data: { type: "removeRecentProject", data: { path: project.path } } } })
												}}
											/>
										</OverflowMenu>
									</div>
								</div>
							</div>
						</ClickableTile>
					{/each}
				</div>
			</div>
		</div>
	</div>
</div>
