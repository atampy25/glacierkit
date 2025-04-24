<script lang="ts">
	import type { QuickStartRequest, ProjectInfo } from "$lib/bindings-types"
	import { event } from "$lib/utils"

	import { help } from "$lib/helpray"
	import {
		Button,
		ClickableTile,
		FluidForm,
		Link,
		Modal,
		OutboundLink,
		OverflowMenu,
		OverflowMenuItem,
		TextInput
	} from "carbon-components-svelte"
	import { FolderAdd, FolderOpen, Folders, LogoDiscord, LogoGithub, Document } from "carbon-icons-svelte"
	import { dialog } from "@tauri-apps/api"
	import { shell } from "@tauri-apps/api"
	import { camelCase, upperFirst } from "lodash"
	import { valid } from "semver"

	const DISCORD_URL = "https://discord.gg/6UDtuYhZP6"
	const GITHUB_URL = "https://github.com/glacier-modding"
	const WIKI_URL = "https://glaciermodding.org/"
	const GITHUB_MOD_TEMPLATE_URL = "https://github.com/new?template_name=smf-mod&template_owner=atampy25"

	export let id: string
	let recent_projects: ProjectInfo[] = []

	let dialog_open = false
	type NewProjectConfig = {
		name: string
		author: string
		version: string
		path: string | null
		valid: boolean
	}

	let new_project_config: NewProjectConfig = {
		name: "My amazing mod",
		author: "you",
		version: "1.0.0",
		path: null,
		valid: false
	}

	let invalid_path = false
	$: invalid_name_empty = new_project_config.name.trim().length === 0
	$: invalid_author_empty = new_project_config.author.trim().length === 0
	$: invalid_semver = valid(new_project_config.version) === null

	$: full_path = (new_project_config.path || "") + "/" + (new_project_config.name || "")
	$: project_id = upperFirst(camelCase(new_project_config.author)) + "." + upperFirst(camelCase(new_project_config.name))

	async function refreshRecentList(){
		await event({ type: "editor", data: { type: "quickStart", data: { type: "refreshRecentList", data: { id } } } })
	}

	export async function handleRequest(request: QuickStartRequest) {

		switch (request.type) {
			case "initialise":
				await refreshRecentList()
				break

			case "refreshRecentList":
				recent_projects = request.data.recent_projects
				break

			case "loadLocalProject":
				await event({ type: "global", data: { type: "loadWorkspace", data: request.data.project } })
				await refreshRecentList()
				await event({ type: "global", data: { type: "removeTab", data: id } })
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
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							if (!new_project_config.valid) {
								dialog_open = true

								// Wait until new_project_config.valid becomes true
								const checkValidity = async () => {
									await new Promise((resolve) => {
										const interval = setInterval(() => {
											if (new_project_config.valid) {
												clearInterval(interval)
												resolve(null)
											}
										}, 100) // Check every 100ms
									})
								}
								await checkValidity()
							}
							if (new_project_config.valid === false) return
							if (new_project_config.path !== null) {
								await event({
									type: "editor",
									data: {
										type: "quickStart",
										data: {
											type: "createLocalProject",
											data: {
												id: id,
												project_id: project_id,
												name: new_project_config.name,
												author: new_project_config.author,
												version: new_project_config.version,
												path: new_project_config.path
											}
										}
									}
								})
							}
						}}
					>
						<FolderAdd title="Local" class="h-5 w-5 mr-2" />
						<span class="text-sm">create locally</span>
					</Link>

					<OutboundLink
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							await shell.open(GITHUB_MOD_TEMPLATE_URL)
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
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							const selected = await dialog.open({
								directory: true,
								multiple: false
							})
							if (typeof selected === "string") {
								await event({ type: "global", data: { type: "loadWorkspace", data: selected } })
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
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							await shell.open(DISCORD_URL)
						}}
					>
						<LogoDiscord title="Discord" class="h-5 w-5 mr-2" />
						<span class="text-sm">discord server</span>
					</OutboundLink>
				</div>
				<div class="flex flex-col">
					<OutboundLink
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							await shell.open(GITHUB_URL)
						}}
					>
						<LogoGithub title="Github" class="h-5 w-5 mr-2" />
						<span class="text-sm">github org</span>
					</OutboundLink>
				</div>
				<div class="flex flex-col">
					<OutboundLink
						class="flex items-center py-1 cursor-pointer"
						on:click={async () => {
							await shell.open(WIKI_URL)
						}}
					>
						<Document title="Wiki" class="h-5 w-5 mr-2" />
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
					{#if recent_projects.length === 0}
						<p class="text-sm pl-2">No recent projects found.</p>
						<p class="text-sm pl-2">You can open or create a project from the menu on the left.</p>
					{/if}
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
												on:click={async (e) => {
													e.stopPropagation()
													await event({ type: "editor", data: { type: "quickStart", data: { type: "openProjectInExplorer", data: { path: project.path } } } })
												}}
											/>
											<OverflowMenuItem
												danger
												text="Remove recent"
												on:click={async (e) => {
													e.stopPropagation()
													await event({ type: "global", data: { type: "removeRecentProject", data: project.path } })
													await event({ type: "editor", data: { type: "quickStart", data: { type: "refreshRecentList", data: { id } } } })
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

	<Modal
		bind:open={dialog_open}
		preventCloseOnClickOutside
		selectorPrimaryFocus="#proj-name"
		modalHeading="Create project"
		primaryButtonText="Confirm"
		secondaryButtonText="Cancel"
		on:click:button--secondary={() => (dialog_open = false)}
		on:click:button--primary={() => {
			if (invalid_name_empty) return
			if (invalid_author_empty) return
			if (invalid_semver) return
			if (new_project_config.path === null) {
				invalid_path = true
				return
			}
			new_project_config.valid = true
			dialog_open = false
		}}
		on:open
		on:close
		on:submit
	>
		<FluidForm>
			<TextInput id="proj-name" required invalid={invalid_name_empty} labelText="Project name"
					   bind:value={new_project_config.name} invalidText="Project name cannot be empty" />
			<br />
			<div class="flex gap-4">
				<TextInput class="flex-grow" required invalid={invalid_author_empty} labelText="Project author" bind:value={new_project_config.author}
						   invalidText="Invalid author! Please don't use special characters" />
				<TextInput
					class="w-32"
					required
					invalid={invalid_semver}
					labelText="Project version"
					bind:value={new_project_config.version}
					invalidText="Invalid version! Please use a semver format"
				/>
			</div>
			<br />
			<div class="flex">
				<TextInput
					class="flex-grow"
					required
					invalid={invalid_path}
					labelText="Project location"
					bind:value={full_path}
					placeholder="C:/your/path/here/{new_project_config.name ?? 'My mod'}"
					invalidText="Project path cannot be empty"
				/>
				<Button
					class="flex-none"
					icon={Folders}
					iconDescription="Select in explorer"
					aria-label="Select project location"
					tooltipPosition="left"
					tooltipAlignment="end"
					on:click={async () => {
						const selected = await dialog.open({
							directory: true,
							multiple: false
						})
						if (typeof selected === "string") {
							new_project_config.path = selected
							invalid_path = false
						}
					}}
				/>
			</div>
			<br />
			<TextInput labelText="project id" readonly bind:value={project_id}></TextInput>
		</FluidForm>
	</Modal>
</div>
