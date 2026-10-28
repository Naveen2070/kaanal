# `crates/kaanal-core/src/splice.rs`

Edits to a config text that change only the bytes they touch (Studio step 2, part 2c).

## What it does

The Studio draft is the text of a `kaanal.yaml`, and the UI changes it with path-plus-value edits. Re-serializing a parsed document would drop the comments, the order of keys and the flow style (`{ from: a, to: b }`) the project uses. So an edit is one replacement of a byte range found from the parser's spans, and every other byte stays as it was, the BOM and the line endings included. Edits apply one after the other, each on the result of the last (the text is parsed again before each).

The module builds its own small span tree from the `saphyr-parser` events. Two facts about those events shape it: `Marker.index` is a **character** index (it is converted to bytes, so text before a non-ASCII character does not shift an edit), and a block mapping or sequence reports an empty span, so a block collection's end is the end of its last child.

## Types and functions

| Item | Kind | What it does |
| --- | --- | --- |
| `Edit::Set { path, value }` | enum variant | Replaces the value at `path`. A scalar (quotes included) or a flow collection is replaced in place; a block collection or an empty value (`key:`) is written after its key's colon. |
| `Edit::Insert { parent, key, value }` | enum variant | Adds `key: value` to the mapping at `parent` (empty path: the root). The key is written bare when that reads back as the same text (letters, digits, `_`, `-`, not a number or a word like `yes`), double-quoted (JSON escapes) otherwise, so a key can never carry syntax; `Exists` compares the key's value, so a quoted `"a b"` is found. Flow map: `, key: value` after the last entry (an empty `{}` becomes `{ key: value }`). Block map: a new line after the last entry's line, indented like the first key, before any trailing comments. An empty document gets its first key at the end. |
| `Edit::Remove { path }` | enum variant | Removes a mapping entry or list item: whole lines in a block collection, the entry and its comma in a flow one (the last entry leaves `{}`). |
| `Spliced { text, lost }` | struct | The edited text, and one line for each edit that was skipped and each comment a removal left without its entry. |
| `SpliceError` | enum | `Parse` (not YAML this module reads, or a non-scalar key), `NotFound(path)` and `Exists(path)` (inserting a key that is there). The text is not changed. |
| `splice(src, edits)` | fn | Applies the edits in order. A BOM is kept; new lines use the file's own line ending (CRLF if it has one). |
| `scalar_paths(src)` | fn | The scalar leaves with their paths and their text exactly as written (quotes included). |

## What is refused, not guessed

A value with an anchor (`&a`), one that is an alias (`*a`), or anything inside an anchored value is not edited (changing it would change its copies), and neither is a shape this module does not handle: a block collection inside a list, a key on a `- ` line that holds more, an empty block mapping as an insert target. Each is left alone and listed in `Spliced.lost`, so the caller can ask the user. Comments directly above a removed entry stay and are listed too; so is a comment between `key:` and a block value that a `Set` replaces, and a `Remove` of the only entry of a block mapping (its parent is left empty, null).

Keys are matched by their value (quotes and escapes resolved), not by their source text, so `"a": 1` is found by `a`. Two equal keys in one mapping are `SpliceError::Parse` (`duplicate key 'a'`): the document is ambiguous, so no edit is guessed. A block scalar (`|`, `>`) is replaced from its key's colon to the end of its text, like a block collection, and its text is not listed by `scalar_paths`. Its span, and an empty value's, are corrected from what the parser reports (they run into the next line), so removing or inserting next to them never takes the following entry.

## Tests

[../../tests.md](../../tests.md#crateskaanal-coretestssplicers--edits-that-keep-the-rest-of-the-text) — span replacement, insert (flow, block, root, empty document, no final newline), comments, CRLF and BOM, non-ASCII, anchors, removal, errors, and the "set every leaf to itself" round trip.
