<script lang="ts">
	import { Button, Search } from "carbon-components-svelte"
	import { type ComponentProps, createEventDispatcher } from "svelte"
	import { Asterisk, Quotes, CharacterSentenceCase } from "carbon-icons-svelte"
	import type { SearchQuery } from "$lib/bindings"

	let { size, query = $bindable(), ...rest }: { query: SearchQuery } & ComponentProps<Search> = $props()

	const dispatch = createEventDispatcher()
</script>

<div class="flex gap-x-2">
	<Search {size} {...rest} bind:value={query.data} on:input on:change on:clear />
	{#if query.type === "raw"}
		<Button
			size={(
				{
					sm: "small",
					lg: "field",
					xl: "lg"
				} as const
			)[size || "lg"]}
			icon={Quotes}
			iconDescription="Exact text"
			on:click={() => {
				query.type = "simple"
				dispatch("input", query.data)
				dispatch("change", query.data)
			}}
		/>
	{:else if query.type === "simple"}
		<Button
			size={(
				{
					sm: "small",
					lg: "field",
					xl: "lg"
				} as const
			)[size || "lg"]}
			icon={CharacterSentenceCase}
			iconDescription="Space-separated terms"
			on:click={() => {
				query.type = "regex"
				dispatch("input", query.data)
				dispatch("change", query.data)
			}}
		/>
	{:else if query.type === "regex"}
		<Button
			size={(
				{
					sm: "small",
					lg: "field",
					xl: "lg"
				} as const
			)[size || "lg"]}
			icon={Asterisk}
			iconDescription="Regular expression"
			on:click={() => {
				query.type = "raw"
				dispatch("input", query.data)
				dispatch("change", query.data)
			}}
		/>
	{/if}
</div>
