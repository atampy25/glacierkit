<script lang="ts">
	import { CodeSnippet, TextInput } from "carbon-components-svelte"
	import { v4 } from "uuid"
	import md5 from "md5"
	import { Decimal } from "decimal.js"

	export async function handleRequest(request: any) {
		// Text transformer is purely frontend
	}

	let uuid = v4()
	let pathToCalculateHash = ""
	let stringToCalculateLocHash = ""
</script>

<div class="w-full h-full p-6 overflow-y-auto">
	<h4 class="mb-2">Random UUID</h4>
	<CodeSnippet
		code={uuid}
		on:copy={() => {
			uuid = v4()
		}}
	/>

	<h4 class="mt-4 mb-2">Hash calculator</h4>
	<TextInput bind:value={pathToCalculateHash} placeholder="[assembly:/_pro/characters/templates/hero/agent47/agent47.template?/agent47_default.entitytemplate].pc_entitytype" />
	<div class="mt-4">
		<div class="bx--label">Hex</div>
		<CodeSnippet code={("00" + md5(pathToCalculateHash).slice(2, 16)).toUpperCase()} />
		<br />
		<div class="bx--label">Decimal</div>
		<CodeSnippet code={new Decimal("0x" + md5(pathToCalculateHash).slice(2, 16)).toString()} />
	</div>

	<h4 class="mt-4 mb-2">Localisation hash calculator</h4>
	<TextInput bind:value={stringToCalculateLocHash} placeholder="UI_SOME_TEXT" />
	<div class="mt-4">
		<div class="bx--label">Hex</div>
		<CodeSnippet code={window.crc.crc32(stringToCalculateLocHash).toString(16).toUpperCase()} />
		<br />
		<div class="bx--label">Decimal</div>
		<CodeSnippet code={window.crc.crc32(stringToCalculateLocHash).toString()} />
	</div>
</div>
