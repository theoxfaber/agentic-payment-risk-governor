# Contributing

## The gate

Every PR must pass the same checks CI enforces — run them locally first:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`-D warnings` means exactly that: a single warning fails the build.

## Test policy

- Every feature or fix lands **with the tests that pin its behavior**, in the
  same commit.
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each crate's
  `lib.rs`; pipeline-level behavior lives in `governor/tests/`.
- `cargo test --workspace` must be fully offline: no network, no credentials,
  no Docker. Tests that need live infrastructure (NATS via compose) are tagged
  `#[ignore = "reason"]` and run explicitly.
- When you fix a bug, add the regression test first (red), then fix (green),
  and reference the failure story in `docs/BUGS.md`.

## Conventions

- Conventional commit messages: `feat:`, `fix:`, `test:`, `ci:`, `docs:`,
  `chore:`. One logical change per commit.
- Typed errors via `thiserror` per crate; no unwrapped `String` errors across
  boundaries.
- Safety property to keep in mind when touching the decision combiner
  (`action-service/src/lib.rs`): a high risk score with contradictory or
  low-confidence evidence must escalate to REVIEW, never auto-BLOCK or
  auto-ALLOW. The tests in `action-service` and `governor/tests/
  investigated_decisions.rs` enforce this — extend them if you change combiner
  semantics.

## Local run

See README "Run it" — `cargo run -p governor-server` gives you the API plus
dashboard on `http://127.0.0.1:8080`.
