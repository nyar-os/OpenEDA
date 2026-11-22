# Development

Maintainer guide for building, testing and releasing OpenEDA.

## Environment

- Node.js 24 and pnpm 10.28 (the versions CI uses).
- A current stable Rust toolchain with `rustfmt` and `clippy`; CI uses nightly.
- Windows, macOS or Linux. The native addon is built for the host unless a
  `--target` is given.

```bash
pnpm install
```

## Build

`wae.config.ts` fixes the build order: native → TypeScript → UI → skills →
staging.

```bash
pnpm run build          # the full pipeline
pnpm run build:napi     # only the open-eda-napi addon for the host
pnpm run build:ts       # only @wae/open-eda (dist/*.js)
pnpm run build:ui       # only the static studio
pnpm run stage:native   # copy addons into the platform packages
```

The addon is not committed. It is staged as
`open-eda.<platform>-<arch>.node` into `projects/packages/open-eda-<platform>-<arch>`
and resolved at runtime from `process.platform` and `process.arch`. Without the
matching platform package, engine calls report `capability/not-implemented`;
only local debugging sets `OPEN_EDA_NATIVE` to a `.node` file.

The studio build emits `projects/packages/open-eda-ui/dist/static`, which is
copied to `projects/packages/open-eda/dist/gui`.

## Tests

```bash
# Rust model and addon
cargo test -p open-eda-model -p open-eda-napi --offline

# TypeScript (compiles src + tests into dist-test and runs node:test)
pnpm --filter @wae/open-eda run test

# Typecheck, including tests
pnpm --filter @wae/open-eda run typecheck
```

Test layout:

- `projects/crates/eda-model/tests/unit/` — private-model tests. `src/lib.rs`
  and the module files include them through
  `#[cfg(test)] #[path = "../tests/unit/<name>.rs"] mod tests;`, so they can
  assert internal behavior without living in `src/`.
- `projects/packages/open-eda/tests/` — TypeScript smoke tests compiled with
  `tsconfig.test.json`; native-addon cases skip when no addon is installed.

Do not put test modules or `*.test.ts` files under `src/`.

## Formatting, linting and CI

```bash
cargo fmt -p open-eda-model -p open-eda-napi
cargo clippy -p open-eda-model -p open-eda-napi --all-targets -- -D warnings
pnpm typecheck
pnpm test
```

CI runs on `master` and `dev`:

- `check-rust.yml` — `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` (nightly toolchain).
- `check-typescript.yml` — `pnpm install --frozen-lockfile`, `pnpm typecheck`,
  `pnpm run build:ts` on Node 24.

## Release

The application packages are `private`; the publishable set is the per-platform
native packages produced by staging. `wae.config.ts` leaves `release` unset
while that is true, and there is no CI release workflow yet. Do not commit
generated addons, `dist/`, `dist-test/` or `<file>.history.json`.

## Adding a capability

1. Start in `projects/crates/eda-model`: document shape, validation, edit,
   check, reader or writer.
2. Add the operation in `eda-napi` as a value mapping, then in the SDK
   (`open-eda.ts`, `service.ts`, `types.ts`) and the CLI (`cli.ts` plus
   `locales/en-US/cli.json`).
3. Add a model test under `tests/unit/` and extend the TypeScript smoke test.
4. Update `capabilities()` and `documentation/capabilities.md`; update
   `documentation/cli.md` for a command or flag change.
5. Run the checks above plus `git diff --check`; stage explicit paths.
