<script lang="ts">
	import { T, Canvas, useLoader } from "@threlte/core"
	import { OrbitControls } from "@threlte/extras"
	import { OBJLoader } from "three/addons/loaders/OBJLoader.js"

	export let obj_path: string = ""
	export let bounding_box: [number, number, number, number, number, number] = [-1, -1, -1, 0, 0, 0]
	export let width: number = 200
	export let height: number = 200

	const center = findCenter(bounding_box)
	const objectSize = [bounding_box[3] - bounding_box[0], bounding_box[4] - bounding_box[1], bounding_box[5] - bounding_box[2]]
	const scaleFactor = 1 / Math.max(...objectSize)

	function findCenter(boundingBox: [number, number, number, number, number, number]): [number, number, number] {
		const [minX, minY, minZ, maxX, maxY, maxZ] = boundingBox
		const centerX = (minX + maxX) / 2
		const centerY = (minY + maxY) / 2
		const centerZ = (minZ + maxZ) / 2
		return [centerX, centerY, centerZ]
	}
</script>

<Canvas size={{ width: width, height: height }}>
	<T.PerspectiveCamera makeDefault position={[1, 1, 1]} on:create={({ ref }) => {
		ref.lookAt(center[0], center[1], center[2])
	}}>
		<T.DirectionalLight position={[5, 5, 5]} />
		<OrbitControls enableDamping />
	</T.PerspectiveCamera>
	<T.AmbientLight color={0xaaaaaa} />

	{#await useLoader(OBJLoader).load(obj_path) then obj}
		<T is={obj} position={[-center[0]*scaleFactor, -center[2]*scaleFactor, center[1]*scaleFactor]} rotation={[-1.5707,0,0]} scale={[scaleFactor,scaleFactor,scaleFactor]}>
			<T.MeshPhongMaterial color={0xE7E7E7FF} />
		</T>
	{/await}
</Canvas>
