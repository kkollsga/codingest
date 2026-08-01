import { helper } from "./util"
import { deepThing } from "./nested/deep"

export function consume(n: number): string {
  return deepThing(String(helper(n)))
}
