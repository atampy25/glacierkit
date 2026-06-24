<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { OrbitControls, GLTF } from "@threlte/extras"

	let { src = "", boundingBox = [-1, -1, -1, 0, 0, 0] }: { src: string; boundingBox: [number, number, number, number, number, number] } = $props()

	const center = $derived(getCenter(boundingBox))
	const objectSize = $derived([boundingBox[3] - boundingBox[0], boundingBox[4] - boundingBox[1], boundingBox[5] - boundingBox[2]])
	const scaleFactor = $derived(1 / Math.max(...objectSize))

	function getCenter(boundingBox: [number, number, number, number, number, number]): [number, number, number] {
		const [minX, minY, minZ, maxX, maxY, maxZ] = boundingBox
		const centerX = (minX + maxX) / 2
		const centerY = (minY + maxY) / 2
		const centerZ = (minZ + maxZ) / 2
		return [centerX, centerY, centerZ]
	}
</script>

<Canvas>
	<T.PerspectiveCamera
		makeDefault
		position={[1, 1, 1]}
		on:create={({ ref }) => {
			ref.lookAt(center[0], center[2], -center[1])
		}}
	>
		<T.DirectionalLight position={[5, 5, 5]} />
		<OrbitControls enableDamping />
	</T.PerspectiveCamera>

	<T.AmbientLight color={0xaaaaaa} />

	<GLTF url={src} position={[-center[0] * scaleFactor, -center[2] * scaleFactor, center[1] * scaleFactor]} rotation={[0, 0, 0]} scale={[scaleFactor, scaleFactor, scaleFactor]} />
</Canvas>
