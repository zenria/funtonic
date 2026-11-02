# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Funtonic started as a test project for [tonic](https://github.com/hyperium/tonic) and is now a command & control / remote command execution system for Unix boxes. A central **taskserver** (the master) dispatches commands to **executors** running on remote machines. Operators use the **commander** CLI to launch commands and admin operations. All communication is gRPC over mTLS.

## Conventions

- Code, identifiers, comments and doc comments are in **English**.
- Write the most idiomatic Rust possible: `?` over `unwrap`, `thiserror` for library error types, `anyhow` at binary/application boundaries, iterator chains over manual loops, `From`/`TryFrom` for conversions (see the existing protobuf ⇄ domain conversions in `common/src/executor_meta.rs`).
- Shared dependency versions live in `[workspace.dependencies]` in the root `Cargo.toml`; crates reference them with `{ workspace = true }`.

## Commands

```sh
cargo build --all --all-targets      # build everything (what CI runs)
cargo test                           # all tests, including integration tests
cargo test -p query-parser           # tests of one crate
cargo test -p integration tls_test   # a single test
cargo fmt --all
cargo clippy --all-targets
```

- `grpc-service/build.rs` generates `grpc-service/src/grpc_protocol.rs` from `proto/tasks/*.proto` (gitignored, written into `src/`, not `OUT_DIR`). `protoc` is built from source via `protobuf-src`, so a working `cmake` is required and the first build is slow.
- CI (`.gitlab-ci.yml`) builds static binaries for `x86_64-unknown-linux-musl`; tagged releases push `executor`, `taskserver` and `commander`.
- Integration tests (`integration/src/lib.rs`) spin up a real taskserver, executor and commander in-process on fixed ports (54010, 54011, …) and load certs from `integration/tls/` via paths relative to the crate dir. They rely on `sleep`s, so they are slow and can be flaky under load; port collisions mean they cannot run alongside another instance.

## Workspace layout

| Crate | Role |
|---|---|
| `grpc-service` | Protobuf definitions (`proto/tasks/`) and generated tonic code. Re-exports `tonic` and `prost`. Its crate version is the **protocol version**. |
| `query-parser` | `nom` parser for executor-targeting queries (`*`, `pattern`, `field:sub_field:pattern`, `and`/`or`/`not`, parentheses) plus the `QueryMatcher` trait. |
| `exec` | Async child-process execution streaming stdout/stderr lines as `ExecEvent`s. |
| `common` (package name **`funtonic`**) | Core library: config types/loading, TLS setup, ed25519 crypto & keystores, executor metadata/tags, and the whole `TaskServer` implementation (`common/src/task_server/`). Re-exports `tonic`, `prost`, `tokio`. |
| `taskserver`, `executor`, `commander` | Thin binaries (each has `lib.rs` with a `*_main` function + `main.rs`), so the integration tests can drive them in-process. |
| `integration` | End-to-end tests only. |

Note that the `common/` directory is imported as `funtonic::…`.

## Architecture

### Flow

1. The executor connects and calls `ExecutorService.GetTasks` (server-streaming). The taskserver keeps an `mpsc` sender per connected executor in `TaskServer::executors`.
2. The commander calls `CommanderService.LaunchTask` with a target query + signed payload. The taskserver matches the query against the persisted executor metadata (`known_executors.yml`, rustbreak), registers a task sink in `tasks_sinks`, and forwards the payload to each matching executor's stream.
3. The executor runs the command and streams `TaskExecution` results back; the taskserver routes them by task id to the commander's `LaunchTaskResponse` stream. The first response message is always `MatchingExecutors`.

### Security model (two layers)

- **Transport**: mTLS for every connection (`TlsConfig` in `common/src/config.rs`). TLS is optional in config (plain TCP otherwise).
- **Application**: every command is a `SignedPayload` (protobuf bytes + nonce + `valid_until_secs`, signed with ed25519 over `payload || nonce_le || valid_until_le`, see `common/src/crypto/signed_payload.rs`). The taskserver verifies commander signatures against `authorized_keys` / `admin_authorized_keys`, and **each executor re-verifies** the signature against its own `authorized_keys` before executing.
- Executors authenticate with their own ed25519 key (`executor_ed25519_key.yml`, auto-generated next to `executor.yml`). Unknown executor keys land in the taskserver's `unapproved_executors_keys.yml` and the executor receives no task until an admin approves it (`commander admin …`), which moves it to `trusted_executors_keys.yml`.
- There is no end-to-end encryption: the taskserver sees commands and outputs in clear.

### Executor lifecycle

`executor_main` loops forever with backoff on errors. `AuthorizeKey` / `RevokeKey` tasks make `executor_main` return an updated `ExecutorConfig`; `executor/src/main.rs` then rewrites `executor.yml` and reconnects. This is how key changes are persisted on executors.

### Configuration

Config files are resolved from `--config`, then `~/.funtonic/`, then `/etc/funtonic/` (`server.yml`, `executor.yml`, `commander.yml`). The taskserver and executor log via `log4rs-gelf`, reading `/etc/funtonic/*-log4rs.yaml` and falling back to `<crate>/assets/log4rs.yaml` (so run them from the workspace root during development). `commander utils genkey <name>` generates an ed25519 key pair in config-ready YAML.

### Protocol changes

Edit `grpc-service/proto/tasks/*.proto` and rebuild. When the wire protocol changes, bump the `grpc-service` version. Executors send it as `clientProtocolVersion`, and the taskserver rejects executors whose protocol version differs from its own (`common/src/task_server/executor_service_impl.rs`), so the taskserver and executors must be upgraded together.
