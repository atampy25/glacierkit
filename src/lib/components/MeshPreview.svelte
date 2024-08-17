<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { OrbitControls } from "@threlte/extras"
	import { OBJLoader } from "three/addons/loaders/OBJLoader.js"
	import { DEG2RAD } from "three/src/math/MathUtils.js"

	export let obj = ""
	export let boundingBox: [number, number, number, number, number, number] = [-1, -1, -1, 0, 0, 0]

	const center = getCenter(boundingBox)
	const objectSize = [boundingBox[3] - boundingBox[0], boundingBox[4] - boundingBox[1], boundingBox[5] - boundingBox[2]]
	const scaleFactor = 1 / Math.max(...objectSize)

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
			ref.lookAt(center[0], center[1], center[2])
		}}
	>
		<T.DirectionalLight position={[5, 5, 5]} />
		<OrbitControls enableDamping />
	</T.PerspectiveCamera>

	<T.AmbientLight color={0xaaaaaa} />

	<T
		is={new OBJLoader().parse(obj)}
		position={[-center[0] * scaleFactor, -center[2] * scaleFactor, center[1] * scaleFactor]}
		rotation={[-90 * DEG2RAD, 0, 0]}
		scale={[scaleFactor, scaleFactor, scaleFactor]}
	>
		<T.MeshPhongMaterial color={0xe7e7e7ff} />
	</T>
</Canvas>
