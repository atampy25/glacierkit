<script lang="ts">
	import { Button, ComposedModal, ModalHeader, ModalBody, ModalFooter, TextInput } from "carbon-components-svelte"

	import CloseOutline from "carbon-icons-svelte/lib/CloseOutline.svelte"
	import AddAlt from "carbon-icons-svelte/lib/AddAlt.svelte"

	import { createEventDispatcher } from "svelte"

	const dispatch = createEventDispatcher<{ updated: string[] }>()

	export let data: string[]

	let newValueInputModalOpen = false
	let newValue = ""
</script>

<table class="table-auto border-collapse bg-[#393939]">
	<tbody>
		{#each data as value, index (value)}
			<tr class:border-b={index != data.length - 1} class="border-solid border-b-white">
				<td class="py-2 px-4 text-[#f4f4f4]">
					<div class="flex flex-row gap-4 items-center">
						<code class="flex-grow break-all">{value}</code>
						<Button
							kind="ghost"
							size="small"
							icon={CloseOutline}
							iconDescription="Remove value"
							on:click={() => {
								data = data.filter((a) => a !== value)
								dispatch("updated", data)
							}}
						/>
					</div>
				</td>
			</tr>
		{/each}
		{#if data.length == 0}
			<tr class="border-solid border-b-white">
				<td class="py-2 px-4 text-[#f4f4f4]">
					<div class="flex flex-row gap-4 items-center">
						<code class="flex-grow">No entries</code>
					</div>
				</td>
			</tr>
		{/if}
	</tbody>
</table>
<br />
<div class="text-white">
	<Button
		kind="primary"
		icon={AddAlt}
		on:click={() => {
			newValueInputModalOpen = true
		}}
	>
		Add an entry
	</Button>
</div>

<ComposedModal
	open={newValueInputModalOpen}
	on:submit={() => {
		newValueInputModalOpen = false
		data = [...data, newValue]
		newValue = ""

		dispatch("updated", data)
	}}
>
	<ModalHeader title="Add an entry" />
	<ModalBody>
		<TextInput labelText="Entry to add" bind:value={newValue} />
	</ModalBody>
	<ModalFooter primaryButtonText="Continue" />
</ComposedModal>
