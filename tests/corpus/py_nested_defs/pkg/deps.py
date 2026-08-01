"""Plain top-level callables — the resolvable targets for everything in
``factories.py``. Nothing here is nested; it exists so a nested definition's
calls have somewhere real to land."""


def audit(tag):
    return tag


def emit(value):
    return value


def notify(state):
    return state
