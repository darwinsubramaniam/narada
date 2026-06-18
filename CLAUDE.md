# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Primary reference: AGENTS.md

**Read [AGENTS.md](./AGENTS.md) first.** It is the authoritative guide for writing code here. It documents the Dioxus 0.7 API surface this project targets — components, signals/state, `use_resource`, routing, fullstack server functions, and hydration. Dioxus 0.7 changed every API: `cx`, `Scope`, and `use_state` no longer exist. Follow AGENTS.md, not older Dioxus knowledge.

This file covers only what AGENTS.md does not: build/run commands and the workspace's cross-crate architecture.

## Commands

This is a [Dioxus](https://dioxuslabs.com/) project; the build/serve tool is the `dx` CLI (install: `curl -sSL http://dioxus.dev/install.sh | sh`).

Serve a platform app (hot-reloads; runs the bundled server for server functions):

```bash
dx serve --package web        # or: desktop, mobile
```

Standard cargo also works, but platform crates have `default = []` features, so you must enable a renderer feature explicitly (the platform feature matches the crate name):

```bash
cargo check   -p web --features web          # type-check the web client build
cargo check   -p web --features server       # type-check the server build
cargo clippy  -p ui                          # lint (see clippy.toml below)
cargo test    -p api                          # run a crate's tests
cargo test    -p api echo                     # run a single test by name filter
```

## Architecture

Cargo workspace with a crate per platform plus two shared crates. Dependency direction is one-way:

```
web / desktop / mobile   (platform entry points, one binary each)
        └── ui           (shared components)
              └── api     (shared server functions)
```

- **`packages/api`** — all fullstack server functions (e.g. `#[post("/api/echo")]`). Async fns that run only on the server and are called like normal async fns from the client. The server-side body is compiled only when the `server` feature is on.
- **`packages/ui`** — components shared across every platform (`Hero`, `Navbar`, `Echo`). Keep platform-specific dependencies *out* of this crate. Components here call into `api` server functions (see `echo.rs`).
- **`packages/web` / `desktop` / `mobile`** — each is a thin entry point (`main.rs`) that owns its **own** `Route` enum and a platform-specific `*Navbar` wrapper around the shared `ui::Navbar`. The `Route` enum is per-platform on purpose so views can diverge per platform; route view components live in each crate's `src/views/`.

### Feature flags (important)

Every platform crate exposes two axes of features:

- A **renderer** feature named after the crate: `web` → `dioxus/web`, `desktop` → `dioxus/desktop`, `mobile` → `dioxus/mobile`.
- A **`server`** feature that cascades down the dependency graph: `web/server` enables `dioxus/server` + `ui/server` → `api/server`.

Because of this, the same source compiles into two distinct binaries (client renderer vs. server). When something only builds in one mode, check which feature you're compiling with.

## Lint rule to respect (clippy.toml)

`clippy.toml` forbids holding a Dioxus signal/generational borrow across an `await` point (`GenerationalRef`, `GenerationalRefMut`, `dioxus_signals::WriteLock`). Doing so deadlocks reads/writes while the future is pending. Read a signal, drop the borrow, *then* await — never hold `.read()` / `.write()` results across `.await`.
