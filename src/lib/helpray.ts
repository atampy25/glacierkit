import type { Action } from "svelte/action"

export const help: Action<HTMLElement, { title: string; description: string }> = (node, params) => {
	node.setAttribute("data-helpray", JSON.stringify(params))
}
