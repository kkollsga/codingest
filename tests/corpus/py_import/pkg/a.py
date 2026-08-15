from . import util
from .util import helper as h
from .sub.deeper import deep_thing


def a_fn():
    return h() + util.helper() + deep_thing()
