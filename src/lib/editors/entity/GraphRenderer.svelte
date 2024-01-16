<script lang="ts">
	export let curveToPreview: [number, number, number, number, number, number, number, number][]

	let canvas: HTMLCanvasElement

	$: {
		if (canvas && curveToPreview) {
			const ctx = canvas.getContext("2d")

			if (!ctx) {
				throw new Error("No canvas??")
			}

			const minX = canvas.width * 0.025
			const maxX = canvas.width - canvas.width * 0.025
			const minY = canvas.height - canvas.height * 0.1
			const maxY = canvas.height * 0.1

			ctx.fillStyle = "white"
			ctx.fillRect(0, 0, canvas.width, canvas.height)

			ctx.fillStyle = "#262626"
			ctx.strokeStyle = "#262626"
			ctx.lineWidth = 0.4

			ctx.beginPath()
			ctx.moveTo(0, canvas.height - canvas.height * 0.1)
			ctx.lineTo(canvas.width, canvas.height - canvas.height * 0.1)
			ctx.stroke()

			ctx.moveTo(canvas.width * 0.025, canvas.height - maxY)
			ctx.lineTo(canvas.width * 0.025, 0)
			ctx.stroke()

			const minXValue = curveToPreview[0][0]
			const maxXValue = curveToPreview.at(-1)![0]

			const points: Record<number, [number, number][]> = {}

			for (const pixel of new Array(Math.round(maxX - minX)).keys()) {
				const xPos = minXValue + ((maxXValue - minXValue) / (maxX - minX)) * pixel

				let curveIndex = curveToPreview.findLastIndex((a) => xPos > a[0])

				if (curveIndex < 0) {
					curveIndex = 0
				}

				const curveToEvaluate = curveToPreview[curveIndex]

				const yVal =
					curveToEvaluate[2] * Math.pow((xPos - curveToEvaluate[0]) / (curveToPreview[curveIndex + 1] ? curveToPreview[curveIndex + 1][0] - curveToEvaluate[0] : 1), 5) +
					curveToEvaluate[3] * Math.pow((xPos - curveToEvaluate[0]) / (curveToPreview[curveIndex + 1] ? curveToPreview[curveIndex + 1][0] - curveToEvaluate[0] : 1), 4) +
					curveToEvaluate[4] * Math.pow((xPos - curveToEvaluate[0]) / (curveToPreview[curveIndex + 1] ? curveToPreview[curveIndex + 1][0] - curveToEvaluate[0] : 1), 3) +
					curveToEvaluate[5] * Math.pow((xPos - curveToEvaluate[0]) / (curveToPreview[curveIndex + 1] ? curveToPreview[curveIndex + 1][0] - curveToEvaluate[0] : 1), 2) +
					curveToEvaluate[6] * ((xPos - curveToEvaluate[0]) / (curveToPreview[curveIndex + 1] ? curveToPreview[curveIndex + 1][0] - curveToEvaluate[0] : 1)) +
					curveToEvaluate[7]

				points[curveIndex] ??= []
				points[curveIndex].push([xPos, yVal])
			}

			const maxYValue = Object.values(points)
				.flat()
				.reduce((prev, cur) => (cur[1] > prev ? cur[1] : prev), -Infinity)

			const minYValue = Object.values(points)
				.flat()
				.reduce((prev, cur) => (cur[1] < prev ? cur[1] : prev), Infinity)

			ctx.fillText(String(Math.round(minXValue * 1000) / 1000), minX, minY + maxY / 2)
			ctx.fillText(String(Math.round(maxXValue * 1000) / 1000), maxX - minX / 2, minY + maxY / 2)
			ctx.fillText(String(Math.round(minYValue * 1000) / 1000), minX - minX / 1.2, minY - maxY / 4)
			ctx.fillText(String(Math.round(maxYValue * 1000) / 1000), minX - minX / 1.2, maxY)

			let ind = 0
			for (const [curve, data] of Object.entries(points)) {
				for (const [curvePoint, [pointX, pointY]] of data.entries()) {
					if (curvePoint === 0 && +curve != -1) {
						ctx.fillStyle = "#262626"
						ctx.fillText(
							String(`(${Math.round(curveToPreview[+curve][0] * 1000) / 1000}, ${Math.round(curveToPreview[+curve][1] * 1000) / 1000})`),
							ind + minX,
							minY - ((minY - maxY) / (maxYValue - minYValue)) * (pointY - minYValue) - maxY / 6
						)
					}

					ctx.fillStyle = [...new Array(15).keys()].flatMap((a) => ["green", "blue", "red"])[+curve]

					ctx.fillRect(ind + minX, minY - ((minY - maxY) / (maxYValue - minYValue)) * (pointY - minYValue), 1, 1)

					ind++
				}
			}
		}
	}
</script>

<canvas bind:this={canvas} width={window.innerWidth * 0.4} height={window.innerHeight * 0.2} />
