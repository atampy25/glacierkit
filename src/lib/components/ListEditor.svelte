<script lang="ts">
	import { Button, ComposedModal, ModalHeader, ModalBody, ModalFooter, TextInput } from "carbon-components-svelte"

	import CloseOutline from "carbon-icons-svelte/lib/CloseOutline.svelte"
	import AddAlt from "carbon-icons-svelte/lib/AddAlt.svelte"

	import { createEventDispatcher } from "svelte"

	const dispatch = createEventDispatcher<{ updated: string[] }>()

	export let data: string[]

	let newValueInputModalOpen = false
	let newValue = ""

	let entryToAddInput: HTMLInputElement
</script>

<div class="flex flex-col gap-1 mb-2">
	{#each data as value}
		<div class="flex items-center gap-2">
			<div class="p-2 bg-[#393939] text-[#f4f4f4] flex-grow">
				<code style="font-size: 0.95em" class="break-all">{value}</code>
			</div>
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
	{/each}
	{#if data.length == 0}
		<div class="p-2 bg-[#393939] text-[#f4f4f4] flex items-center gap-2">
			<code style="font-size: 0.95em">No entries</code>
		</div>
	{/if}
</div>
<Button
	size="small"
	kind="primary"
	icon={AddAlt}
	on:click={() => {
		newValueInputModalOpen = true
	}}
>
	Add an entry
</Button>

<ComposedModal
	open={newValueInputModalOpen}
	on:close={() => {
		newValueInputModalOpen = false
	}}
	on:submit={() => {
		newValueInputModalOpen = false
		data = [...data, newValue]
		newValue = ""

		dispatch("updated", data)
	}}
>
	<ModalHeader title="Add an entry" />
	<ModalBody>
		<TextInput
			labelText="Entry to add"
			data-modal-primary-focus
			bind:value={newValue}
			bind:ref={entryToAddInput}
			on:keydown={({ key }) => {
				if (key === "Enter") {
					if (newValue.length > 0) {
						newValueInputModalOpen = false
						data = [...data, newValue]
						newValue = ""

						dispatch("updated", data)
						entryToAddInput.value = ""
					}
				}
			}}
		/>
	</ModalBody>
	<ModalFooter primaryButtonText="Continue" />
</ComposedModal>
