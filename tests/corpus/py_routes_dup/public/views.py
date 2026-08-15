from flask import Blueprint

public = Blueprint("public", __name__)


@public.route("/")
def index():
    """The public landing page — registered at '/' with no methods= kwarg."""
    return "public"


@public.route("/about")
def about():
    return "about"


# Multi-line decorator pin (2026-08-15): the dominant real-world FastAPI
# style breaks the line after `(`. Before the newline fix in
# first_string_literal this registration produced NO Route node at all —
# 20/147 lost on the first real-repo acceptance run.
@app.route(
    "/multiline",
    methods=["POST"],
)
def multiline_route():
    return "pinned"
