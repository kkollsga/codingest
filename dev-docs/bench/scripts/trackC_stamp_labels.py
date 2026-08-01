#!/usr/bin/env python3
"""Stamp the `verdict` column on the Track C CALLS label set.

The verdicts below were produced ONCE, by reading each callee's definition and
each caller's call site in the opencode checkout pinned at
1e17856ba4b5b052650c8115060852f3f023844e. They are the fixed ground truth that
Phases 4–6 re-score against; re-labeling is only needed if the pin changes.

Labeled names: fetch, use, close, value, provide — `fetch` (the headline case)
plus the four largest names that fit the ~400-row budget, chosen to span BOTH
emit paths named in the plan's open question #1: `fetch`/`use`/`close` are
fan-out sites (2–4 surviving candidates each), while `value`/`provide` reach
their callee through a lower-fan-out path.

WHY EACH CALLEE'S EDGES ARE FALSE BY DEFAULT (definition read at the pin):

  packages/function/src/api.SyncServer.fetch
      `async fetch()` (api.ts:20) — a method on the SyncServer Durable Object,
      reachable only through a DO stub (`stub.fetch(...)`, api.ts:173, which is
      a same-file site the resolver never emitted). Every labeled caller writes
      either a bare `fetch(...)` (the WHATWG global), `app.fetch(...)` (Hono's
      handler) or `net.fetch(...)` (Electron's net module).

  packages/opencode/src/mcp/catalog.fetch
      `export function fetch<T>(...)` (catalog.ts:85). No labeled caller is in
      catalog.ts and none names it (`Catalog.fetch`); all are the global.

  packages/llm/src/route/auth.value
      `export const value = (secret, source) => ...` (auth.ts:85), re-exported
      as `Auth.value` through the `@opencode-ai/llm/route` barrel. Bare
      `value()` sites in the labeled set are Solid render-prop closures
      (`{(value) => ... value()}`) and `Redacted.value` is Effect's.

  packages/cli/src/framework/runtime.provide
      `function provide(node, handlers)` (runtime.ts:62) — NOT exported, so
      only same-file callers can be true. `Layer.provide` / `Effect.provide` /
      `Scope.provide` are Effect's.

  packages/opencode/src/cli/cmd/run/footer.RunFooter.close
      `public close(): void` (footer.ts:619) — a method; only `this.close()` /
      `footer.close()` inside footer.ts are true. `ws.close` / `server.close` /
      `socket.close` / `rl.close` / `dialog.close` are unrelated objects.

  script/github/close-issues.close
      `async function close(num)` (close-issues.ts:42) — module-private.

  packages/opencode/src/effect/instance-state.use
      `export const use = (self, select) => ...` (instance-state.ts:52), called
      as `InstanceState.use`. `Database.use` / `Config.Service.use` /
      `Workspace.Service.use` are other objects' `use` (none of which is any of
      the four candidates), and `ctx.use()` in worker-pool.tsx is the object
      returned by `createSimpleContext`.

  packages/sdk/js/src/{,v2/}gen/client/utils.gen.Interceptors.use
      `use(fn: Interceptor)` (utils.gen.ts:242) — a method on the Interceptors
      class. True only at `client.interceptors.<kind>.use(...)`, and only for
      the copy the caller's own client actually builds: `src/client.ts` imports
      `./gen/client/client.gen.js` (v1), `src/v2/client.ts` imports
      `./gen/client/client.gen.js` relative to `v2/` (v2).

  packages/tui/src/routes/session.use
      `function use()` (session/index.tsx:172) — module-private; true only for
      the bare `use()` sites inside session/index.tsx.

Usage: trackC_stamp_labels.py <labels.csv>
"""
import csv
import sys

# (caller, callee) pairs whose edge is a TRUE call. Everything else is false.
# Each entry carries the site that proves it.
TRUE_EDGES = {
    # same-file call to a module-private function
    ("packages/cli/src/framework/runtime.run",
     "packages/cli/src/framework/runtime.provide"),                      # runtime.ts:59 `provide(commands, handlers)`
    ("script/github/close-issues.main",
     "script/github/close-issues.close"),                                # close-issues.ts:90,101 `await close(num)`
    # same-file method call through `this`
    ("packages/opencode/src/cli/cmd/run/footer.RunFooter.constructor",
     "packages/opencode/src/cli/cmd/run/footer.RunFooter.close"),        # footer.ts:351 `this.close()`
    # cross-package call through the workspace barrel
    ("packages/core/src/session/runner/model.apiKey",
     "packages/llm/src/route/auth.value"),                               # model.ts:8 imports { Auth } from "@opencode-ai/llm/route"; :84-87 `Auth.value(...)`
    # cross-file method call, callee file reached transitively (client.gen -> utils.gen)
    ("packages/sdk/js/src/client.createOpencodeClient",
     "packages/sdk/js/src/gen/client/utils.gen.Interceptors.use"),       # client.ts:54-55 `client.interceptors.request.use(...)`
    ("packages/sdk/js/src/v2/client.createOpencodeClient",
     "packages/sdk/js/src/v2/gen/client/utils.gen.Interceptors.use"),    # v2/client.ts:78-91
}

# Every caller inside session/index.tsx calling the module-private `use()`.
SESSION_USE = "packages/tui/src/routes/session.use"
SESSION_FILE = "packages/tui/src/routes/session/index.tsx"


def verdict(row: dict) -> str:
    if (row["caller"], row["callee"]) in TRUE_EDGES:
        return "true"
    if (
        row["callee"] == SESSION_USE
        and row["caller_file"] == SESSION_FILE
        and row["receivers"] == "<bare>"
    ):
        return "true"
    return "false"


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    path = sys.argv[1]
    rows = list(csv.DictReader(open(path)))
    for row in rows:
        row["verdict"] = verdict(row)
    with open(path, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    t = sum(1 for r in rows if r["verdict"] == "true")
    print(f"{len(rows)} rows: {t} true / {len(rows) - t} false -> {path}")

    # Sanity: every hand-listed TRUE pair must have matched a real row —
    # a typo in the table would otherwise silently shrink the truth set.
    matched = {(r["caller"], r["callee"]) for r in rows if r["verdict"] == "true"}
    missing = TRUE_EDGES - matched
    if missing:
        sys.exit(f"TRUE_EDGES entries matched no row: {missing}")


if __name__ == "__main__":
    main()
