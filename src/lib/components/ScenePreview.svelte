<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { Studio } from "@threlte/studio"
	import { OrbitControls, useGltf } from "@threlte/extras"
	import SceneGeometry from "./SceneGeometry.svelte"
	import { Box3, Group, Vector3 } from "three"
	import { onMount } from "svelte"
	import { convertFileSrc } from "@tauri-apps/api/core"

	let {
		geometry,
		assets,
		editorId
	}: {
		geometry: {
			geometry: Record<
				string,
				{
					value:
						| {
								type: "Spatial"
								parent: { idx: number } | null
								transform: [number, number, number, number, number, number, number, number, number, number, number, number]
						  }
						| {
								type: "Geometry"
								parent: { idx: number } | null
								transform: [number, number, number, number, number, number, number, number, number, number, number, number]
								scale: [number, number, number] | null
								prim: string
						  }
				} | null
			>
		}
		assets: Record<string, string>
		editorId: string
	} = $props()

	let sceneGroup: Group | undefined = $state()
	let position: [number, number, number] = $state([0, 0, 0])

	onMount(() => {
		const interval = setInterval(() => {
			if (sceneGroup) {
				const box = new Box3().setFromObject(sceneGroup, true)
				const center = new Vector3()
				box.getCenter(center)

				if (Math.abs(position[0] - center.x) > 0.1 || Math.abs(position[1] - center.y) > 0.1 || Math.abs(position[2] - center.z) > 0.1) {
					position = [center.x, center.y, center.z]
				}
			}
		}, 500)

		return () => {
			clearInterval(interval)
		}
	})
</script>

<Canvas>
	<T.PerspectiveCamera makeDefault position={[position[0] + 5, position[1] + 5, position[2] + 5]}>
		<OrbitControls enableDamping target={[position[0], position[1], position[2]]} />
	</T.PerspectiveCamera>

	<T.AmbientLight color={0xaaaaaa} />

	<T.Group bind:ref={sceneGroup}>
		{#each Object.entries(geometry.geometry) as [key, value]}
			{#if value?.value && !value?.value?.parent}
				<SceneGeometry geom={key} allGeom={geometry.geometry} {assets} {editorId} />
			{/if}
		{/each}
	</T.Group>
</Canvas>
