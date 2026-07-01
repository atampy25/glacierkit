<script lang="ts">
	import { T } from "@threlte/core"
	import { useGltf } from "@threlte/extras"
	import SceneGeometry from "./SceneGeometry.svelte"
	import { Matrix4 } from "three"
	import { convertFileSrc } from "@tauri-apps/api/core"

	let {
		geom,
		allGeom,
		assets,
		editorId
	}: {
		geom: string
		allGeom: Record<
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
		assets: Record<string, string>
		editorId: string
	} = $props()

	const data = $derived(allGeom[geom]!.value)

	const gltf = $derived(data.type === "Geometry" && data.prim ? useGltf(convertFileSrc(`${editorId}/${assets[data.prim]}`, "editor-asset")) : null)

	function zUpToYUp(
		mat: [number, number, number, number, number, number, number, number, number, number, number, number, number, number, number, number]
	): [number, number, number, number, number, number, number, number, number, number, number, number, number, number, number, number] {
		return [
			// Column 0 (X)
			mat[0],
			mat[2],
			-mat[1],
			mat[3],

			// Column 1 (Y)
			mat[8],
			mat[10],
			-mat[9],
			mat[11],

			// Column 2 (Z)
			-mat[4],
			-mat[6],
			mat[5],
			-mat[7],

			// Column 3 (Translation)
			mat[12],
			mat[14],
			-mat[13],
			mat[15]
		]
	}
</script>

<T.Group
	oncreate={(ref) => {
		const mat = zUpToYUp([
			data.transform[0],
			data.transform[1],
			data.transform[2],
			0,
			data.transform[3],
			data.transform[4],
			data.transform[5],
			0,
			data.transform[6],
			data.transform[7],
			data.transform[8],
			0,
			data.transform[9],
			data.transform[10],
			data.transform[11],
			1
		])
		ref.applyMatrix4(new Matrix4().fromArray(mat))
	}}
>
	{#if data.type === "Geometry"}
		{#if gltf && $gltf}
			<T.Group
				oncreate={() => {
					for (const [_, material] of Object.entries($gltf.materials)) {
						material.metalness = 0.2
					}
				}}
			>
				{@const clone = $gltf.scene.clone()}
				<T is={clone} scale={data.scale ?? [1, 1, 1]} />
			</T.Group>
		{:else}
			<T.Mesh>
				<T.BoxGeometry args={data.scale ?? [1, 1, 1]} />
				<T.MeshStandardMaterial color={0x00ff00} wireframe />
			</T.Mesh>
		{/if}
	{/if}
	{#each Object.entries(allGeom) as [key, value]}
		{#if value?.value?.parent?.idx === +geom}
			<SceneGeometry geom={key} {allGeom} {assets} {editorId} />
		{/if}
	{/each}
</T.Group>
