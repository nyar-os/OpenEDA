# AGENTS.md

Guidance for agents working in the OpenEDA repository.

## Purpose

OpenEDA keeps one editable, revisioned `.oeda` schematic document and exposes it
to people, scripts and agents. Schematic semantics (edits, checks, diff,
statistics and every reader/writer) live in the Rust model. The SDK, CLI, studio
and agent skill are surfaces over that model.

## Documentation map

| File | Audience |
|------|----------|
| [README.md](README.md) | Product landing page |
| [projects/packages/open-eda/README.md](projects/packages/open-eda/README.md) | npm registry page |
| [documentation/architecture.md](documentation/architecture.md) | Module boundaries and data flow |
| [documentation/capabilities.md](documentation/capabilities.md) | Capability matrix, checks, formats, losses, boundaries |
| [documentation/cli.md](documentation/cli.md) | Command and flag reference |
| [documentation/development.md](documentation/development.md) | Build, test, staging, CI and release |
| [documentation/README.md](documentation/README.md) | Maintainer doc index |

## Architecture invariants

1. **The Rust model is the only implementation.** Document shape, validation,
   edits, checks, diff, statistics and every reader/writer live in
   `projects/crates/eda-model`. Do not add a second schematic implementation.
2. **`eda-napi` maps values only.** Each export accepts a path or JSON request
   and returns JSON. It must not add domain rules, defaults that change results,
   or caching.
3. **The TypeScript SDK/CLI do not duplicate domain logic.** `cli.ts` parses
   arguments, calls one session or `Design` method, prints the result and returns
   an exit code. Unsupported work maps to exit code `2`.
4. **The capability matrix is the only support claim.** `capabilities()` is the
   source of truth; do not claim a format, check or export target that is not
   `supported` there, and add a test before promoting one.
5. **Unsupported work fails explicitly.** Return `capability/not-implemented`
   instead of approximating silently.
6. **Edits persist atomically.** Every editing call writes through a temporary
   file and a rename. Undo history never enters the document.
7. **Reports stay deterministic.** The same document and command produce
   byte-identical output; Markdown/HTML contain no timestamps.
8. **Tests live in `tests/`.** Use `tests/unit/` with `#[path]` for private Rust
   tests and `tests/` with `tsconfig.test.json` for TypeScript. Never put test
   modules or `*.test.ts` under `src/`.

## Verified commands

```bash
pnpm install
cargo fmt -p open-eda-model -p open-eda-napi
cargo test -p open-eda-model -p open-eda-napi --offline
cargo clippy -p open-eda-model -p open-eda-napi --offline --all-targets -- -D warnings
pnpm --filter @wae/open-eda run typecheck
pnpm --filter @wae/open-eda run test
```

`pnpm run build` performs the full native → TypeScript → UI → skills → staging
pipeline. The addon is not committed.

## Change checklist

- Search the repository before assuming a command exists; verify flags against
  `projects/packages/open-eda/src/cli.ts`.
- Add or update a test for every behavioral change.
- Keep the model, Node-API surface, SDK types and CLI in sync when a capability
  changes.
- Update `capabilities()` and `documentation/capabilities.md`; update
  `documentation/cli.md` for a command or flag change.
- Update the package README only for registry-facing changes (exports, usage,
  constraints). Keep product narrative in the root README.
- Never put build, monorepo, publish or architecture detail in either README.
- Run the checks above plus `git diff --check` before committing.
- Stage explicit paths; never `git add -A`. Do not push unless asked.

## Commit style

Use a one-line subject with an emoji prefix:

- `✨ Add ...` — new capability or user-visible feature
- `🐛 Fix ...` — bug fix
- `📝 ...` — documentation, comments or prose
- `🧪 ...` — test-only change

Keep the subject imperative and specific. If a change spans documentation and
behavior, split it into separate commits.
