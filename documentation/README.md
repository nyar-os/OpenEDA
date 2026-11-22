# OpenEDA documentation

Maintainer and reference documentation. The user-facing landing page is the
repository [README](../README.md); the npm registry page is the
[`@wae/open-eda` package README](../projects/packages/open-eda/README.md).

| Document | Contents |
|----------|----------|
| [architecture.md](architecture.md) | Schematic model, module boundaries, data flow, determinism, errors, history ownership |
| [capabilities.md](capabilities.md) | Capability matrix, check semantics, exchange formats, loss codes, unsupported boundaries |
| [cli.md](cli.md) | Full `open-eda` command reference with flags and a worked example |
| [development.md](development.md) | Environment, build, tests, native addon staging, CI and release |
| [AGENTS.md](../AGENTS.md) | Rules for agents working in this repository |

## Source of truth

The Rust `open-eda-model` crate in `projects/crates/eda-model` is the only
implementation of document I/O, validation, edits, checks and writers. The
Node-API crate and the TypeScript SDK/CLI only map values and arguments onto it.
When this documentation and the code disagree, the code wins; update the
document in the same change.
