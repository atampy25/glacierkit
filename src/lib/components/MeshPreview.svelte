<script lang="ts">
	import { T, Canvas } from "@threlte/core"
	import { OrbitControls, GLTF } from "@threlte/extras"
	import { WebIO } from "@gltf-transform/core"
	import { metalRough } from "@gltf-transform/functions"
	import { KHRONOS_EXTENSIONS } from "@gltf-transform/extensions"

	async function transform(url: string) {
		const io = new WebIO().registerExtensions(KHRONOS_EXTENSIONS)
		const doc = await io.read(url)
		await doc.transform(metalRough())
		return URL.createObjectURL(new Blob([await io.writeBinary(doc)]))
	}

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

	let objectUrl: string | null = $state(null)

	$effect(() => {
		if (src) {
			void transform(src).then((url) => {
				objectUrl = url
			})
		}

		return () => {
			if (objectUrl) {
				URL.revokeObjectURL(objectUrl)
				objectUrl = null
			}
		}
	})
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

	{#if objectUrl}
		<GLTF url={objectUrl} position={[-center[0] * scaleFactor, -center[2] * scaleFactor, center[1] * scaleFactor]} rotation={[0, 0, 0]} scale={[scaleFactor, scaleFactor, scaleFactor]} />
	{/if}
</Canvas>
