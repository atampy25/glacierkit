import type { Ref } from "$lib/bindings-types"

export const genRandHex = (size: number) => [...Array(size)].map(() => Math.floor(Math.random() * 16).toString(16)).join("")

/** Get the local entity ID referenced by a Ref. If the reference is external, returns false. If the reference is null, returns null. */
export function getReferencedLocalEntity(ref: Ref) {
	if (ref !== null && typeof ref != "string" && ref.externalScene) {
		return false // External reference
	} else {
		return ref !== null && typeof ref != "string" ? ref.ref : ref // Local reference
	}
}

/** Returns a modified Ref that points to a given local entity, keeping any exposed entity reference the same */
export function changeReferenceToLocalEntity(ref: Ref, ent: string): Ref {
	if (typeof ref == "string" || ref === null) {
		return ent
	} else {
		return {
			ref: ent,
			externalScene: null,
			exposedEntity: ref.exposedEntity
		}
	}
}
