---
name: notify
description: Send a coordination/feedback note to another local project's inbox. Resolves a target repo by name anywhere under the Koding/ parent tree, composes a message file per the inbox schema, and drops it in that repo's inbox/unread/ (creating the folder if missing).
---

# notify

Deliver a message to a sibling project's inbox so its maintainer/agent picks it
up on their next `read-inbox`. Input: a **target repo** (name or path) and the
**message** (topic + body; compose from the conversation if not given).

The most frequent target is **KGLite** — codingest's upstream. When a
parity-relevant bug lives in the shared `code_tree` logic, or the standalone
transform needs an affordance from `kglite-mcp-server` / `kglite::api` (e.g. the
MCP build-hook), that's an *upstream ask*: KGLite source is read-only from this
workspace, so you **notify**, you don't patch.

## 1. Resolve the target repo path
The target lives somewhere under the `Koding/` parent (category folders like
`Rust/`, `Go/`, `JS/`, `mcp-servers/`; repos sit at depth 1–2, sometimes
deeper). Search by name (case-insensitive):

```bash
KODING="${PWD%%/Koding/*}/Koding"
find "$KODING" -maxdepth 3 -type d -iname '<name>' \
  -not -path '*/node_modules/*' -not -path '*/.git/*' \
  -not -path '*/__pycache__/*' -not -path '*/target/*' \
  -not -path '*/mcp-servers/*'
```

- **KGLite** resolves to `<KODING>/Rust/KGLite` — a sibling of this repo. Its
  inbox is `<KODING>/Rust/KGLite/inbox/unread/`. Writing a note there is *not* a
  source edit, so it respects the read-only rule.
- **`mcp-servers/` is one externally-managed project, not a tree of repos.** Its
  subdirs (`code_review/`, `open_source/`, …) are **not** notify targets —
  that's why `*/mcp-servers/*` is excluded above. To reach anything in that
  ecosystem, target **`mcp-servers`** itself → its single `inbox/unread/`. Never
  resolve a name to `mcp-servers/<subdir>/`.
- **Exactly one match** → use it.
- **Several matches** → prefer a git repo (has `.git/`); if still ambiguous,
  **ask the user which path** (show the candidates).
- **No match** → widen with `-maxdepth 4`, then ask the user for the path.
- If the caller gave an absolute path directly, skip the search and use it.

Confirm the resolved path before writing if there was any ambiguity.

## 2. Ensure the inbox exists
```bash
mkdir -p "<target>/inbox/unread"
```
(Create it if the project has no inbox yet — expected for a first note.)

## 3. Compose the message (the schema)
Filename: **`<YYYY-MM-DD>-from-codingest-<topic-slug>.md`** (date = session date,
kebab-case topic). Body:

```markdown
# <Short title>

- **From:** codingest
- **To:** <target repo>
- **Date:** <YYYY-MM-DD>
- **Type:** feedback | bug | coordination | heads-up | request
- **Re:** <optional — version, file, PR, or prior message it responds to>

<1–3 paragraphs of context: what happened / what's needed and why.>

## Ask / action requested
- <concrete, actionable item(s) — or "FYI, no action needed">

## References
- <links, file paths, commit SHAs, versions — optional>
```

Keep it actionable: only file a note if there's something for them to do or
genuinely useful to know (AGENTS.md "Inbox hygiene" — route a note to another
project only if it carries an actionable task for *them*). For an upstream-parity
ask to KGLite, cite the exact `code_tree` / `kglite::api` site and what codingest
needs.

## Send discipline — the receiving side is a person

Every note lands as triage work in someone's `read-inbox` — in this estate,
often the same person operating both ends. The bar is not "true and relevant";
it is **"changes what the recipient does."** Four rules (doctrine 0.1.3):

- **Batch per target, per session.** Non-urgent items accumulate and go as ONE
  combined note when the thread or work session completes — never one note per
  finding. An immediate single-purpose note is justified only by: a **blocker**
  (work here cannot proceed), a **reply the target explicitly requested**, or a
  **time-sensitive coordination fact**.
- **No FYI-grade notes.** If the sender didn't ask for a reply and the content
  doesn't change their next action, the acknowledgment belongs in *your* copy's
  Status footer at archive time (read-inbox step 5), not in their `unread/`.
  Read "genuinely useful to know" narrowly: a bare ack, a "done on our side",
  or a progress report all fail it.
- **Ping a stalled thread at most once, with new evidence.** A ping is
  justified only when the silence blocks live work, and it must carry something
  the original note didn't — a number, a repro, a commit. A bare "any update?"
  is noise.
- **Piggyback related items.** A new small item for a target with an open
  thread rides the next legitimate note to them, not its own file.

## 4. Write + report
Write the file to `<target>/inbox/unread/<filename>` and report the full path.
Don't move or touch anything in our own inbox — this skill only *sends*.

## Notes
- Keep the response under 400 tokens.
- This is the send side; `read-inbox` is the receive side. Same filename schema
  (`YYYY-MM-DD-from-<sender>-<topic>.md`) so the recipient's triage just works.
- Sending writes into another project's working tree — if the resolved target
  was ambiguous, confirm with the user before writing. (Writing to a project's
  gitignored `inbox/` is fine even when its *source* is read-only to us.)
