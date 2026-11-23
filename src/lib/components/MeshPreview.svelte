<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { OrbitControls, GLTF, Align, Grid, Gizmo } from "@threlte/extras"
	import { BackSide } from "three"

	let { src = "" }: { src: string } = $props()

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
		<T.DirectionalLight position={[5, 5, 5]} />
		<OrbitControls enableDamping target={[0, centerY, 0]}>
			<Gizmo
				y={{ label: "Z" }}
				nz={{
					label: "Y",
					enabled: true,
					opacity: 1,
					scale: 0.7,
					labelColor: 0x222222,
					line: true,
					border: { size: 0, color: 0xdddddd },
					hover: {
						color: 0xffffff,
						labelColor: 0x222222,
						opacity: 1,
						scale: 0.7,
						border: { size: 0, color: 0xdddddd }
					}
				}}
				z={{ line: false, scale: 0.45, hover: { scale: 0.5 }, label: "" }}
			/>
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
		{#if src}
			<GLTF url={src} />
		{/if}
		<T.Mesh position={[0, 0, 0]}>
			<T.SphereGeometry args={[0.05]} />
			<T.MeshBasicMaterial color="blue" />
		</T.Mesh>
		<T.Mesh position={[0, 0, 0]}>
			<T.SphereGeometry args={[0.06]} />
			<T.MeshBasicMaterial color="white" side={BackSide} />
		</T.Mesh>
	</Align>
	<Grid infiniteGrid sectionColor="white" sectionThickness={1} cellColor="gray" />
</Canvas>
