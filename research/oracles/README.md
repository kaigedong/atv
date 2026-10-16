# Research oracles only

This directory is not an Android, desktop, Cargo, or Gradle build input.
`generate-auth-oracle.cjs` executes pinned original JS/WASM only to regenerate
synthetic differential fixtures. It does not make service calls. Production
libraries and ordinary Rust tests do not execute it or require Node.

Original artifacts must be placed in the ignored research locations documented
in `docs/rust-auth.md`. The extracted historical resolver belongs at
`research/native-auth/live-resolver.vendor.js`, the OpenAPI/ticket WASMs at
`research/native-auth/`, and the ticket JS wrapper at `research/upstream/`.
Run from the repository root only for an explicit algorithm comparison:

```sh
node research/oracles/generate-auth-oracle.cjs
```

The result is `native/crates/atv-auth/tests/oracle-fixtures.json`; all inputs are
synthetic. Those committed fixtures are compared by native Rust tests. The old
WebView app, extraction scripts, npm manifest and original development history
remain local-only; they are not included in the public repository.

`inventory-wasm-interfaces.py` reads the retained REA reports and pinned original
local artifacts to regenerate `wasm-interfaces.json`. It verifies byte/WAT
identities and complete interface coverage, without executing WASM or fetching
resources. The JSON contains all typed imports/exports and static JS forwarding
aliases, not the original code/data or proof of all function semantics. See
`docs/wasm-interfaces.md` for the distinctions and evidence IDs.
