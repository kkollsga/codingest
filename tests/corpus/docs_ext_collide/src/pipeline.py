"""Symbols this corpus's docs mention.

The names are deliberately distinctive so the docs pass's conservative
resolver matches them by unique bare name (none is in ``docs::STOP_WORDS``,
and none is declared twice).
"""


def build_retry_plan(attempts):
    """Return a retry plan."""
    return {"attempts": attempts}


def drain_backlog(plan):
    """Drain the backlog under ``plan``."""
    return plan["attempts"]


def shadowed_only_symbol(marker):
    """Mentioned ONLY by the dropped `.md` collider.

    If the lower-precedence `guide.md` were ever ingested, this would gain a
    MENTIONS edge and the corpus golden would move. That is the point: the
    collision policy is pinned by the golden, not only by a unit test.
    """
    return marker
