from flask import Blueprint

public = Blueprint("public", __name__)


@public.route("/")
def index():
    """The public landing page — registered at '/' with no methods= kwarg."""
    return "public"


@public.route("/about")
def about():
    return "about"
