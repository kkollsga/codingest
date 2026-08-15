from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .a import a_fn

try:
    import json
except ImportError:
    json = None


def c_fn():
    import functools

    return functools.reduce(lambda x, y: x + y, [1, 2])
