---
title: Retry Guide (md, LOSES)
audience: nobody
---

# Retry Guide (md)

The lower-precedence half of the collision: `.mdx` beats `.md`, so this file is
dropped from doc-node emission entirely and contributes NOTHING to the graph.

Its mention of `shadowed_only_symbol` and its link to the
[pipeline](../src/pipeline.py) are the probes — both would show up in the
golden if this file were ingested, and neither may.
