# Overview

A plain `.md` sibling. Its link to the [guide](./guide.mdx) must resolve to the
`.mdx` doc node, which only works because the doc id is extension-stripped the
same way for both flavours.

The repo's [readme](../README.MD) is a doc too: link classification strips a
markup extension the same case-insensitive way `discover_docs` matched it, so
an upper-cased `.MD` destination reaches the `README` Doc node rather than
being filed as a source File.
