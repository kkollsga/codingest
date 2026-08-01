// D3: a cross-file caller. `driver` and `useCounter` are top level and
// resolve normally; `increment`, `reset`, `scrub`, `register` and
// `connectRemote` are closure-scoped in service.ts and must NOT be reachable
// from here — a nested definition never joins the global name lookup.
import { driver, useCounter } from "./service";

export function consume(): string {
  const counter = useCounter(2);
  increment(1);
  reset();
  register("a");
  connectRemote("http://example.test");
  return driver();
}
