"""D3 — a closure-scoped definition resolves for callers in its own file only.

Every nested name this file mentions is declared inside a function in
``factories.py`` and is unreachable from here in real Python. None of these
may produce an edge: not a CALLS edge, not a REFERENCES_FN edge, and not a
DECORATES edge.
"""

from pkg.factories import make_counter


def register(callback):
    return callback


def drive():
    bump(1)
    wrapper()
    normalize("x")
    adapter(2)
    deepest()
    return make_counter(0)


def wire():
    # `wrapper` is globally unique by short name, which is exactly what the
    # REFERENCES_FN index resolves on — so this is the reference that would
    # leak into another file's closure without the D3 gate.
    return register(wrapper)


@decorate
def handler():
    # Same story for DECORATES: `decorate` is nested inside `retrying`.
    return 1
