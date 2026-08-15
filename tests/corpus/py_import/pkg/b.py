import os, sys
import pkg.util as u
from pkg import util
from pkg.sub import deeper


def b_fn():
    return u.helper() + util.helper() + deeper.deep_thing() + len(os.sep) + len(sys.argv)
