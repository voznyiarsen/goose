# AGENTS Instructions

goose is an AI agent framework in Rust with CLI and Electron desktop interfaces.

## Setup
```bash
source bin/activate-hermit
cargo build
```

## Commands

### Build
```bash
cargo build                   # debug
cargo build --release         # release  
just release-binary           # release binary
```

### Test
```bash
cargo test                   # all tests
cargo test -p goose          # specific crate
cargo test --package goose --test mcp_integration_test
just record-mcp-tests        # record MCP
```

### Lint/Format
```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```

### UI
```bash
just run-ui                  # start desktop
cd ui/desktop && pnpm run typecheck
cd ui/desktop && pnpm test   # test UI
```

## Structure
```
crates/
├── goose              # core logic
├── goose-acp-macros   # ACP proc macros
├── goose-cli          # CLI entry
├── goose-mcp          # MCP extensions
├── goose-test         # test utilities
└── goose-test-support # test helpers

ui/desktop/            # Electron app
```

## Development Loop
```bash
# 1. source bin/activate-hermit
# 2. Make changes
# 3. cargo fmt
```

### Run these only if the user has asked you to build/test your changes:
```
# 1. cargo build
# 2. cargo test -p <crate>
# 3. cargo clippy --all-targets -- -D warnings
```

## Rules

- Test: Prefer tests/ folder, e.g. crates/goose/tests/
- Test: When adding features, update goose-self-test.yaml, rebuild, then run `goose run --recipe goose-self-test.yaml` to validate
- Error: Use anyhow::Result
- Provider: Implement Provider trait see providers/base.rs
- MCP: Extensions in crates/goose-mcp/
- UI Desktop: Use ACP SDK types or local `src/types/*` types. Do not import generated OpenAPI types/client code from `ui/desktop/src/api`

## Code Quality

- Comments: Write self-documenting code - prefer clear names over comments
- Comments: Never add comments that restate what code does
- Comments: Only comment for complex algorithms, non-obvious business logic, or "why" not "what"
- Simplicity: Don't make things optional that don't need to be - the compiler will enforce
- Simplicity: Booleans should default to false, not be optional
- Errors: Don't add error context that doesn't add useful information (e.g., `.context("Failed to X")` when error already says it failed)
- Simplicity: Avoid overly defensive code - trust Rust's type system
- Logging: Clean up existing logs, don't add more unless for errors or security events

## Native Terminal UI (`crates/goose-tui`)

- The `goose tui` command is a native Rust TUI built into the `goose` binary (the `goose-tui` crate, default `tui` feature in `goose-cli`). It replaces the old Node/Ink TUI; there is no npx/runtime dependency.

- Architecture: `tui` spawns `goose acp` as a child process and drives it over the Agent Client Protocol (stdio). The ACP connection future is `!Send`, so it runs on a `tokio::task::LocalSet` via `spawn_local`. Session/tool updates arrive as `AcpEvent`s; config and extension operations go through ACP custom requests (`ConfigReadAll`, `ConfigUpsert`, `GetConfigExtensions`, `GetAvailableExtensions`, `AddConfigExtension`, `RemoveConfigExtension`, `SetConfigExtensionEnabled`).

- Rendering uses `ratatui` + `ratatui-textarea`. One logical content line maps to one rendered line; the viewport shows a tail of the current turn and truncates every line to the available width (truncate, never wrap) so manual scroll math stays correct. Respect the truncation-not-wrap discipline: when changing overlay/card dimensions, recompute how many rows fit and cap list items to the available height rather than letting content overflow.

- Reuse ACP for config/extensions — do not re-implement config logic in the TUI. Permission requests from the ACP session are auto-approved. Provider/model switching persists via `GOOSE_PROVIDER`/`GOOSE_MODEL` config upsert followed by `NewSession`; full OAuth/key setup delegates to `goose configure` (temporarily leaves the alternate screen).

## Never

- Never: Recreate `ui/desktop/src/api` or add `@hey-api/openapi-ts` to `ui/desktop`
- Cargo.toml: For human-authored dependency changes, use `cargo add` instead of manually editing dependency entries unless there is a specific reason not to.
- Cargo.toml: Automated dependency bump PRs are exempt; when manual edits are necessary, keep `Cargo.lock` consistent.
- Never: Skip cargo fmt
- Never: Merge without running clippy
- Never: Comment self-evident operations (`// Initialize`, `// Return result`), getters/setters, constructors, or standard Rust idioms

## Entry Points
- CLI: crates/goose-cli/src/main.rs
- UI: ui/desktop/src/main.ts
- Agent: crates/goose/src/agents/agent.rs
