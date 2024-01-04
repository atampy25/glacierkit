<script language="ts">
	import { Button } from "carbon-components-svelte"

	let nuts = 0
</script>

Deeznuts {nuts}

<Button on:click={() => (nuts = nuts + 1)}>Deez</Button>
