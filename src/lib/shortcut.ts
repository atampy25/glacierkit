export const shortcut = (
	node: HTMLElement,
	params: {
		control?: boolean
		shift?: boolean
		alt?: boolean
		key: string
		callback?: (event: KeyboardEvent) => void
	}
) => {
	let handler: (event: KeyboardEvent) => void

	const removeHandler = () => window.removeEventListener("keydown", handler)
	const setHandler = () => {
		removeHandler()

		if (!params) {
			return
		}

		handler = (e) => {
			if (
				params.key == e.key &&
				((!params.control && !e.ctrlKey && !e.metaKey) || (params.control && (e.ctrlKey || e.metaKey))) &&
				((!params.shift && !e.shiftKey) || (params.shift && e.shiftKey)) &&
				((!params.alt && !e.altKey) || (params.alt && e.altKey))
			) {
				e.preventDefault()

				if (params.callback) {
					params.callback(e)
				} else {
					node.click()
				}
			}
		}

		window.addEventListener("keydown", handler)
	}

	setHandler()

	return {
		update: setHandler,
		destroy: removeHandler
	}
}
