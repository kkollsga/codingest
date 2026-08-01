"""Every closure-scoped shape the Python scope walk has to get right.

Nothing below the top level of a Python file was visited before the walk
landed, so all of this was invisible: no nodes, and — worse — the calls inside
it were skipped by the call extractor as well, so they left no trace anywhere.
"""

import functools

from pkg.deps import audit, emit, notify


def retrying(attempts):
    """Decorator factory — the classic two-level Python closure.

    `decorate` is depth 1, `wrapper` depth 2, and `wrapper` carries the
    decorator it was written with.
    """

    def decorate(fn):
        @functools.wraps(fn)
        def wrapper(*args, **kwargs):
            # D4 regression. These two calls used to vanish: `extract_calls`
            # skipped the nested `def` on the theory that it was "node-ified
            # elsewhere", and nothing node-ified it.
            audit(attempts)
            return fn(*args, **kwargs)

        return wrapper

    return decorate


def make_counter(start):
    """Closure-returning factory — the bound name escapes, but D3 still
    resolves it same-file only; see consumer.py."""
    total = start

    def bump(step):
        emit(step)
        return total + step

    return bump


def report(rows):
    """A nested helper, plus a lambda that must NOT name a scope."""

    def normalize(row):
        return row.strip()

    # A `lambda` is one of Python's two unnamed scopes. It contributes no
    # chain segment and gets no node, and because its body is an expression
    # it can never hold a definition — so the call inside it belongs to
    # `report`, which is where the CALLS edge to `normalize` must appear.
    strip_all = lambda row: normalize(row)  # noqa: E731
    # Passing a nested definition by value is a same-file REFERENCES_FN. The
    # byte-identical reference in consumer.py must resolve to nothing.
    emit(normalize)
    return sorted(rows, key=strip_all)


def plugin():
    """A nested definition used as a decorator on its nested sibling — a
    same-file DECORATES edge. consumer.py applies the same name across files
    and must get no edge."""

    def trace(fn):
        return fn

    @trace
    def run():
        return audit("run")

    return run


def pick(flag):
    """Two same-named defs in sibling blocks of one scope — the `#{line}`
    duplicate-qualified-name tie-break. `if` is not a scope in Python, so
    both are `…pick.choose` and the second is the one that moves."""
    if flag:

        def choose():
            return notify("on")

    else:

        def choose():
            return notify("off")

    return choose


def guarded():
    """Block statements are not Python scopes, so they are transparent: a
    `def` inside `try`/`except`/`with` is a direct child of `guarded`, at
    depth 1, not one level deeper."""
    try:

        def load():
            return audit("try")

    except ImportError:

        def load():
            return audit("except")

    with open("/dev/null") as handle:

        def close():
            return handle.close()

    return load, close


class Registry:
    """A method body is a scope like any other."""

    def install(self, hook):
        def adapter(value):
            return hook(emit(value))

        return adapter


def local_class():
    """A function-local class contributes a name segment but no node — and no
    nesting level, because a class body is a namespace, not a closure. Its
    methods are still grammar-named definitions on a fully named chain, so
    they do get nodes and their calls land on them."""

    class Inner:
        def run(self):
            def deepest():
                return audit("deep")

            return deepest()

    return Inner
