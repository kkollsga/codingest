import os, sys
import pkg.util as u
from pkg import util
from pkg.sub import deeper


def b_fn():
    return u.helper() + util.helper() + deeper.deep_thing() + len(os.sep) + len(sys.argv)


# Multi-name pin (2026-08-15, closing the B2 scope cut): one statement, one
# submodule name + one symbol name. `sub` must edge to pkg/sub/__init__.py;
# `util` is ambiguous (module AND commonly a symbol) — here it is the module.
from pkg import sub, util


def b_multi():
    return sub.deeper.deep_thing() + util.helper()
