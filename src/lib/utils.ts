import { type Event, commands } from "$lib/bindings"
import { trace } from "tauri-plugin-log"
import { invoke } from '@tauri-apps/api/core';

// eslint-disable-next-line @typescript-eslint/ban-types
type ObjectEntry<T extends {}> = T extends object
	? { [K in keyof T]: [K, Required<T>[K]] }[keyof T] extends infer E
		? E extends [infer K, infer V]
			? K extends string | number
				? [`${K}`, V]
				: never
			: never
		: never
	: never

// eslint-disable-next-line @typescript-eslint/ban-types
export function typedEntries<T extends {}>(object: T): ReadonlyArray<ObjectEntry<T>> {
	return Object.entries(object) as unknown as ReadonlyArray<ObjectEntry<T>>
}

export async function event(event: Event) {
	trace(`Sending event ${JSON.stringify(event, undefined, "\t")}`)
	await commands.event(event)
}

export function trackEvent(name: string, props?: Record<string, unknown>) {
	void invoke<string>('plugin:aptabase|track_event', { name, props })
}

export const showInFolder = commands.showInFolder
