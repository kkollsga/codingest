// Corpus for the closure-scoped-definitions work (0.1.6), phase 3: the
// nested scope walk (D1 as amended, D2, D3, D4).
//
// Before Phase 3 the TS walk only ever looked at the direct children of the
// program root, so every shape in this file below the top level produced
// nothing at all — and the calls inside a nested named binding were dropped
// rather than mis-attributed. Every shape here exists to be pinned by the
// golden: the ones that must become nodes, and just as importantly the ones
// that must not.
import { decode, make, Service } from "./deps";

// ── the Effect-TS service closure — the motivating shape ─────────────────
// `layer` is a depth-0 factory-wrapped Function (Phase 2). Everything bound
// inside its generator body is depth 1, `parent_scope` = `…service.layer`.
export const layer = Layer.effect(
  Service,
  Effect.gen(function* () {
    const client = yield make();

    const connectRemote = Effect.fn("Service.connectRemote")(function* (
      url: string,
    ) {
      return yield decode(url);
    });

    const connectLocal = (command: string): string => {
      return decode(command);
    };

    // Depth 2: a named arrow inside a named arrow.
    const withRetry = (attempts: number): string => {
      const attempt = (n: number): string => {
        return connectLocal(String(n));
      };
      return attempt(attempts);
    };

    // NOT a Function: a value receiver binds the call's *result*, an array.
    // The Phase 2 narrowing has to keep holding at depth > 0.
    const names = client.map((entry: string) => entry.trim());

    // The named binding inside this anonymous callback must NOT become a
    // node — D1 clause 5. `hidden` has no addressable scope chain, and
    // admitting its class is what took opencode from 10.90 % to 14.15 %.
    client.forEach((entry: string) => {
      const hidden = (label: string): string => {
        return decode(label);
      };
      hidden(entry);
    });

    return { connectRemote, connectLocal, withRetry, names };
  }),
);

// ── module factories: the caught form and the IIFE that is not ───────────
// Caught: a curried factory application, so `store` is a Function and its
// body is a named scope — `create` and `clear` are depth-1 nodes.
export const store = defineStore("entries")(function (seed: string[]) {
  const entries = seed;

  function create(name: string): number {
    return entries.push(name);
  }

  const clear = function (): number {
    return entries.length;
  };

  return { create, clear };
});

// NOT caught, and pinned here so it stays that way: a plain IIFE. The single
// function literal is the *callee*, and D1-3's chain walk covers curried
// callees (`f(…)(fn)`) and call-valued arguments only — an immediately
// invoked literal is neither, so the value has zero literals in its chain.
// That is the measured criterion: the Phase 1 spike counted this shape among
// the 10 654 opencode "call chain with no function literal" exclusions, and
// the +3 062-node ceiling assumes it stays out.
// `registry` is a Constant and `register` / `lookup` get no node.
// Known follow-up, not implemented: give an IIFE module factory a named scope.
export const registry = (function () {
  const entries: string[] = [];

  function register(name: string): number {
    return entries.push(name);
  }

  const lookup = function (name: string): boolean {
    return entries.includes(name);
  };

  return { register, lookup };
})();

// ── a React-hook factory returning a closure ─────────────────────────────
export function useCounter(start: number) {
  let value = start;

  const increment = (step: number): number => {
    value = value + step;
    return value;
  };

  // A nested named arrow whose calls must attach to *it*: before Phase 3
  // `extract_calls` skipped the arrow (it is in NESTED_SCOPES) and nothing
  // node-ified it either, so `increment(…)` and `decode(…)` vanished from
  // the graph entirely.
  const reset = (): string => {
    increment(-value);
    return decode(value);
  };

  return { increment, reset };
}

// ── the duplicate-qualified-name tie-break ───────────────────────────────
// Two `scrub`s in sibling blocks of one scope collide on
// `…service.normalize.scrub`. The first keeps the bare name; the second
// takes a `#{line}` suffix.
export function normalize(input: string, upper: boolean): string {
  if (upper) {
    const scrub = (raw: string): string => {
      return raw.toUpperCase();
    };
    return scrub(input);
  } else {
    const scrub = (raw: string): string => {
      return raw.trimStart();
    };
    return scrub(input);
  }
}

// ── a TS namespace: a name segment, but not a nesting level ──────────────
export namespace Text {
  export function widen(value: string): string {
    return decode(value);
  }
}

// ── a class method body is a scope too ───────────────────────────────────
export class Runner {
  run(count: number): string {
    const step = (n: number): string => {
      return decode(n);
    };
    return step(count);
  }
}

// ── same-file callers, so D3's same-file rule has something to resolve ───
export function driver(): string {
  const counter = useCounter(1);
  normalize("x", true);
  Text.widen("y");
  return String(counter);
}
