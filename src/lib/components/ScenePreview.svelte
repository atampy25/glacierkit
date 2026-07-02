<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { OrbitControls, Grid, Gizmo, Align } from "@threlte/extras"
	import SceneGeometry from "./SceneGeometry.svelte"
	import { BackSide } from "three"

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

	let centerY = $state(0)
	let max = $state([0, 0, 0])
</script>

<Canvas>
	<T.PerspectiveCamera
		makeDefault
		position={[max[0] + 2, max[1] + 2, max[2] + 2]}
		oncreate={(ref) => {
			ref.lookAt(0, centerY, 0)
		}}
	>
		<OrbitControls enableDamping target={[0, centerY, 0]}>
			<Gizmo y={{ label: "Z" }} z={{ label: "Y" }} />
		</OrbitControls>
	</T.PerspectiveCamera>

	<T.AmbientLight color={0xaaaaaa} />

	<Align
		auto
		y={false}
		onalign={(data) => {
			centerY = data.height / 2
			max[0] = data.boundingBox.max.x
			max[1] = data.boundingBox.max.y
			max[2] = data.boundingBox.max.z
		}}
	>
		{#each Object.entries(geometry.geometry)
			.filter(([_, value]) => value?.value && !value.value.parent)
			.map(([key, _]) => key) as key (key)}
			<SceneGeometry geom={key} allGeom={geometry.geometry} {assets} {editorId} />
		{/each}
		<T.Mesh position={[0, 0, 0]}>
			<T.SphereGeometry args={[0.1]} />
			<T.MeshBasicMaterial color="blue" />
		</T.Mesh>
		<T.Mesh position={[0, 0, 0]}>
			<T.SphereGeometry args={[0.12]} />
			<T.MeshBasicMaterial color="white" side={BackSide} />
		</T.Mesh>
	</Align>
	<Grid infiniteGrid sectionColor="white" sectionThickness={1} cellColor="gray" />
</Canvas>
