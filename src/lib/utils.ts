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
