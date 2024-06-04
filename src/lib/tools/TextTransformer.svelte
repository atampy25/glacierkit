<script lang="ts">
	import { CodeSnippet, TextInput } from "carbon-components-svelte"
	import { v4 } from "uuid"
	import md5 from "md5"
	import { Decimal } from "decimal.js"
	import { trackEvent } from "@aptabase/tauri"
	import { help } from "$lib/helpray"

	export async function handleRequest(request: any) {
		// Text transformer is purely frontend
	}

	let uuid = v4()
	let pathToCalculateHash = ""
	let stringToCalculateLocHash = ""
	let hexToDecimal = ""
	let decimalToHex = ""
</script>

<div class="w-full h-full p-6 overflow-y-auto" use:help={{ title: "Text tools", description: "This panel lets you generate UUIDs, calculate hashes and convert between different number formats." }}>
	<h4 class="mb-2">Random UUID</h4>
	<CodeSnippet
		code={uuid}
		on:copy={() => {
			trackEvent("Get random UUID")
			uuid = v4()
		}}
	/>

	<h4 class="mt-4 mb-2">Hash calculator</h4>
	<TextInput
		bind:value={pathToCalculateHash}
		on:change={() => {
			if (pathToCalculateHash) {
				trackEvent("Calculate path hash")
			}
		}}
		placeholder="[assembly:/_pro/characters/templates/hero/agent47/agent47.template?/agent47_default.entitytemplate].pc_entitytype"
	/>
	<div class="mt-4">
		<div class="bx--label">Hex</div>
		<CodeSnippet code={("00" + md5(pathToCalculateHash.toLowerCase()).slice(2, 16)).toUpperCase()} />
		<br />
		<div class="bx--label">Decimal</div>
		<CodeSnippet code={new Decimal("0x" + md5(pathToCalculateHash.toLowerCase()).slice(2, 16)).toString()} />
	</div>

	<h4 class="mt-4 mb-2">Localisation hash calculator</h4>
	<TextInput
		bind:value={stringToCalculateLocHash}
		on:change={() => {
			if (stringToCalculateLocHash) {
				trackEvent("Calculate localisation hash")
			}
		}}
		placeholder="UI_SOME_TEXT"
	/>
	<div class="mt-4">
		<div class="bx--label">Hex</div>
		<CodeSnippet code={window.crc.crc32(stringToCalculateLocHash.toUpperCase()).toString(16).toUpperCase()} />
		<br />
		<div class="bx--label">Decimal</div>
		<CodeSnippet code={window.crc.crc32(stringToCalculateLocHash.toUpperCase()).toString()} />
	</div>

	<h4 class="mt-4 mb-2">Hex to decimal</h4>
	<TextInput
		bind:value={hexToDecimal}
		on:change={() => {
			if (hexToDecimal) {
				trackEvent("Convert hex to decimal")
			}
		}}
		placeholder="0123456789ABCDEF"
	/>
	<div class="mt-4">
		<div class="bx--label">Decimal</div>
		<CodeSnippet code={new Decimal("0x" + (hexToDecimal || "0").toLowerCase()).toString()} />
	</div>

	<h4 class="mt-4 mb-2">Decimal to hex</h4>
	<TextInput
		bind:value={decimalToHex}
		on:change={() => {
			if (decimalToHex) {
				trackEvent("Calculate decimal to hex")
			}
		}}
		placeholder="81985529216486895"
	/>
	<div class="mt-4">
		<div class="bx--label">Hex</div>
		<CodeSnippet code={new Decimal(decimalToHex || "0").toHex().slice(2).toUpperCase()} />
	</div>
</div>
