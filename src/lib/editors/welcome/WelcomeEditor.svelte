<script lang="ts">
	import type { WelcomeTabRequest } from "$lib/bindings-types"
	import { event } from "$lib/utils"

	import { onMount } from "svelte"

	import { help } from "$lib/helpray"
	import { Button, ClickableTile, Tile } from "carbon-components-svelte"
	import { FolderAdd, FolderOpen, LogoGithub } from "carbon-icons-svelte"
	import { Pane, Splitpanes } from "svelte-splitpanes"

	export let id: string


	onMount(async () => {
		await event({
			type: "editor",
			data: {
				type: "welcome",
				data: {
					type: "initialise",
					data: {
						id
					}
				}
			}
		})
	})

	export async function handleRequest(request: WelcomeTabRequest) {
		console.log(`Welcome tab ${id} handling request`, request)
	}
</script>

<div
	class="w-full h-full flex flex-col p-4 overflow-y-auto"
	use:help={{
		title: "Welcome",
		description: "The welcome tab helps you with the first steps when you launch Glacierkit."
	}}
>

<Splitpanes theme="">
    <Pane>
        bruz
    </Pane>
    <Pane>
        <Tile light class="mb-4 h-20">
            <div class="flex items-center justify-between">
                <div>
                    <h4 class="mb-1 text-base font-semibold	">Create a new project</h4>
                    <p class="m-1 text-sm">Get started with Glacierkit by creating a new mod project</p>
                </div>
                <div class="flex items-center justify-items-center space-x-8">
                    <!-- Folder Add Icon Section -->
                    <ClickableTile class="flex flex-col items-center justify-center text-center hover:cursor-pointer bg-gray-100 rounded-lg shadow-lg">
                        <div class="flex items-center justify-center">
                            <FolderAdd title="Local" class="h-5 w-5" />
                        </div>
                        <span class="mt-1 text-sm">local</span>
                    </ClickableTile>
        
                    <ClickableTile class="flex flex-col items-center justify-center text-center hover:cursor-pointer bg-gray-100 rounded-lg shadow-lg">
                        <div class="flex items-center justify-center">
                            <LogoGithub title="Github" class="h-5 w-5" />
                        </div>
                        <span class="mt-1 text-sm">git</span>
                    </ClickableTile>
              
                </div>
            </div>
        </Tile>
        
        <Tile light class="mb-4 h-14">
            <div class="flex items-center justify-between">
                <div class="flex-grow">
                    <h4 class="mb-1 text-base font-semibold	">Open a project</h4>
                    <p class="m-1 text-sm">Open an existing SMF project</p>
                </div>
                <div class="flex items-center justify-items-center space-x-8">
                    <!-- Folder Add Icon Section -->
                    <ClickableTile class="flex flex-col items-center justify-center text-center hover:cursor-pointer bg-gray-100 p-4 rounded-lg shadow-lg">
                        <div class="flex items-center justify-center">
                            <FolderOpen title="Local" class="h-5 w-5" />
                        </div>
                        <span class="mt-1 text-sm">local</span>
                    </ClickableTile>
                </div>
            </div>
        </Tile>
    </Pane>
</Splitpanes>


</div>