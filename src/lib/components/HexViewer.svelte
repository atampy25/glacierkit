<script lang="ts">
	import { VList } from "virtua/svelte"

	let { src }: { src: string } = $props()

	let data = $derived(fetch(src).then((res) => res.bytes()))
</script>

{#await data then data}
	{@const rows = Math.ceil(data.byteLength / 16)}
	{@const hexRows = Array.from({ length: rows }, (_, i) => {
		const start = i * 16
		const end = Math.min(start + 16, data.byteLength)
		const rowData = new Uint8Array(data.slice(start, end))
		return Array.from(rowData).map((byte) => byte.toString(16).padStart(2, "0"))
	})}
	{@const asciiRows = Array.from({ length: rows }, (_, i) => {
		const start = i * 16
		const end = Math.min(start + 16, data.byteLength)
		const rowData = new Uint8Array(data.slice(start, end))
		return Array.from(rowData).map((byte) => (byte >= 32 && byte <= 126 ? String.fromCharCode(byte) : "."))
	})}
	<div class="h-full p-4 bg-neutral-800">
		<div class="flex flex-col h-full">
			<div class="font-semibold flex items-start">
				<div>
					<div>Address</div>
					<code class="opacity-0">{"".padStart(Math.max(data.byteLength.toString(16).length, 7), "0")}</code>
				</div>
				<div class="mx-2.5"></div>
				<code
					>{Array.from({ length: 8 }, (_, i) => i)
						.map((a) => a.toString(16).padStart(2, "0") + " ")
						.join(" ")}</code
				>
				<div class="mx-2"></div>
				<code
					>{Array.from({ length: 8 }, (_, i) => i + 8)
						.map((a) => a.toString(16).padStart(2, "0") + " ")
						.join(" ")}</code
				>
				<div class="mx-3"></div>
				<code>ASCII</code>
			</div>
			<VList class="-mt-1 flex-grow" data={hexRows.map((a, i) => [a, asciiRows[i]])} getKey={(_, i) => i}>
				{#snippet children([hexRow, asciiRow], i)}
					<div class="mb-1.5">
						<code class="font-semibold">{(i * 16).toString(16).padStart(Math.max(data.byteLength.toString(16).length, 7), "0")}</code>
						<div class="inline mx-2"></div>
						{#each hexRow.slice(0, 8) as byte}
							<code class:text-neutral-500={byte === "00"}>{byte + " "}</code>
						{/each}
						<div class="inline mx-1"></div>
						{#each hexRow.slice(8) as byte}
							<code class:text-neutral-500={byte === "00"}>{byte + " "}</code>
						{/each}
						<div class="inline mx-2"></div>
						{#each asciiRow as char}
							<code class:text-neutral-500={char === "."}>{char}</code>
						{/each}
					</div>
				{/snippet}
			</VList>
		</div>
	</div>
{/await}
