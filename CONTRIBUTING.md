# Contributing to Loopy

Thank you for your interest in contributing.

## Building

Loopy is a standard Cargo project (Rust edition 2024):

```console
$ cargo build
$ cargo test
$ cargo run -- start
```

## Code style

This project follows the standard Rust conventions enforced by `rustfmt` and
`clippy`:

```console
$ cargo fmt
$ cargo clippy
```

## Dependencies

Add third-party dependencies to `Cargo.toml` as usual. Keep the dependency set
lean — prefer the standard library and existing dependencies where reasonable.

## Web frontend

The UI lives in `web/` (React + TypeScript + Vite). The built assets in
`web/dist/` are embedded into the binary at compile time. After changing the UI:

```console
$ cd web
$ npm install
$ npm run build
```

Then rebuild the binary (`cargo build --release`) to re-embed the assets.

## Tests

Run `cargo test` before opening a pull request. Frontend tests run with
`cd web && npm test`.
