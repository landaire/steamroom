# Agent guidelines

Engineering style for this repository. Apply to all generated code, docs,
and commits.

## Code style

- No LLM-style decorative punctuation in code or docs. No emdashes, endashes,
  or arrow glyphs. Use ASCII (`-`, `--`, `->`) or rewrite to avoid the dash.
- Comments are for non-obvious things: the "why", a hidden invariant, a
  workaround for a specific bug. If a comment only restates what the code
  does, delete it.
- Do not write documentation for the sake of documentation. Module and item
  docstrings exist when the contract is non-obvious from the signature.
- Dry, technically accurate prose. No marketing language, no superlatives.

## Types

- Newtypes over bare primitives for any value with semantic meaning
  (`AppId`, `DepotId`, `ManifestId`, `JobId`). A bare `u32` that could be
  one of three IDs is a defect.
- Typestates over runtime checks where state transitions are known at
  compile time. The login flow (`SteamClient<Encrypted>` to
  `SteamClient<LoggedIn>`) is the model.
- No bare tuples for anything with more than two fields, or two fields
  that are confusable. Use a struct with named fields.

## Errors

- Typed errors via `thiserror`. Variants encode what went wrong; callers
  match on variants.
- If a caller has to inspect a stringified message to react to an error
  case, the error type is missing a variant. Add it.

## Defaults

- No `unwrap_or`, `unwrap_or_else`, `unwrap_or_default`, or other
  silent fallbacks unless the default is proven to be the correct
  choice for every caller. "Safe" defaults are usually
  hard-to-debug ones; the absence of a value is information, and
  swallowing it hides bugs.
- Prefer propagating `None` / `Err`, or returning a typed error variant
  that names the missing condition. If a default is genuinely correct
  (e.g. zero pages of padding, an empty `Vec` as the identity for
  concatenation), leave a one-line comment explaining why.

## Commits

- `jj` with conventional-commit subjects (`feat(scope): ...`,
  `fix(scope): ...`, `docs(scope): ...`).
- No `Co-Authored-By` trailers.

## Review

- At each milestone (a meaningful chunk of completed work, typically a
  phase in an implementation plan), conduct an adversarial code review
  in a fresh subagent. Fresh context catches things the implementer
  missed.
