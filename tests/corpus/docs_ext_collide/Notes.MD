# Notes

A mixed-case extension: `discover_docs` accepts `.MD` case-insensitively, so
`strip_doc_ext` must too — this node's id has to come out as `Notes`, with no
extension left on it.

The link to the [guide](docs/guide.md) still resolves to the surviving
`docs/guide` node even though the `.md` half of that collision was dropped,
because the link target is compared as an extension-stripped concept id.

See also the [pipeline source](src/pipeline.py).
