from flask import Blueprint

admin = Blueprint("admin", __name__)


@admin.route("/")
def index():
    """The admin landing page — the SAME methodless '/' as public/views.py.

    Under the old (framework, method, path) route identity these two
    registrations collapsed into a single Route node, and the survivor
    reported whichever file the sorted walk reached first.
    """
    return "admin"


@admin.route("/health", methods=["GET"])
def health():
    return "ok"


@admin.route("/dup")
def dup_first():
    """Two registrations of one method+path inside ONE file are one
    registration site: they stay a single Route node with parallel HANDLES
    edges (the id carries the declaring file, deliberately not the line)."""
    return "first"


@admin.route("/dup")
def dup_second():
    return "second"
