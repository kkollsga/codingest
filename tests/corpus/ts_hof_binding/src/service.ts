// Corpus for Phase 2 of dev-docs/plans/closure-scoped-definitions.md:
// depth-0 factory-wrapped bindings + the grammar-vocabulary fixes.
//
// No pre-existing corpus contained a `const` function-literal binding, a
// `function*`, or a factory wrap, so the parity net was blind to this whole
// class of change. Every shape below is here to be pinned by the golden.
import { decode, make, Service } from "./deps";

// ── grammar vocabulary: kinds tree-sitter actually emits ─────────────────
// `function_expression` (D-A) — used to be a Constant.
export const asFnExpr = function (n: number): number {
  return n + 1;
};

// `generator_function` (D-B) — used to be a Constant.
export const asGenExpr = function* (n: number) {
  yield asFnExpr(n);
};

// `generator_function_declaration` (D-C) — used to produce no node at all.
export function* exportedGen(n: number) {
  yield decode(n);
}

function* localGen(n: number) {
  yield n;
}

// ── the narrowed factory unwrap: these become Function nodes ─────────────
// Curried application with a generator literal — the Effect-TS shape.
export const readFile = Effect.fn("Service.readFile")(function* (path: string) {
  return yield decode(path);
});

// Curried application with an arrow literal.
export const cached = memoize("cache-key")((n: number) => n + 1);

// Generator inside a call-valued argument; `wrapped_by` is the outermost
// callee, `Layer.effect`.
export const layer = Layer.effect(
  Service,
  Effect.gen(function* () {
    return yield make();
  }),
);

// Bare-identifier callee with a generator literal.
export const enumerate = wrapGenerator(function* (n: number) {
  yield localGen(n);
});

// ── rejected by the narrowing: these stay Constants ──────────────────────
const users = load();

// A value receiver — the binding is an array, not a function.
export const names = users.map((u) => u.name);

// A chained value receiver (`member.expr`).
export const parts = raw.split(",").map((s) => s.trim());

// An acceptable callee, but uncurried and no generator: binds the result.
export const total = createMemo(() => 1 + 2);

// Two literals in the chain — ambiguous, declined.
export const pair = combine(
  function* () {
    yield 1;
  },
  function* () {
    yield 2;
  },
);

// Zero literals in the chain — an ordinary constant.
export const config = build(1, "two", { three: 3 });

export const LIMIT = 42;

// ── a caller, so the new nodes participate in CALLS ──────────────────────
export function driver(n: number): number {
  const seed = asFnExpr(n);
  readFile(String(seed));
  cached(seed);
  enumerate(seed);
  exportedGen(seed);
  return seed;
}
