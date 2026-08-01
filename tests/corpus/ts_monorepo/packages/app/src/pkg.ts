import { helper } from "@scope/core"
import { deepThing } from "@scope/core/nested/deep"

export function viaPackage(): string {
  return deepThing(String(helper(2)))
}
