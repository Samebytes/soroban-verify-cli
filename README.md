# soroban-verify-cli

Rebuilds a Soroban contract from a public commit in a pinned container, hashes the WASM and compares it
with the on-chain hash. Built around SEP-55; testnet only in this version.

Requires `git`, `docker` and the `stellar` CLI on `PATH`.

```
soroban-verify verify <CONTRACT_ID> --repo <URL> --commit <SHA>
    [--image <IMAGE>] [--package <NAME>] [--network testnet] [--json] [--no-cache] [--cache-dir <DIR>]
```

- **Workspaces:** passes when the on-chain hash matches any built contract; `--package` builds one.
- **Toolchain:** `--image` wins; otherwise the image is selected from the `soroban-sdk` version in
  `Cargo.lock` (two official `stellar/stellar-cli` images, pinned by digest).
- **Cache:** build results are cached by repo, commit, image and package (`--no-cache` to skip,
  `--cache-dir` or `SOROBAN_VERIFY_CACHE` to move it). The on-chain hash is always fetched fresh.
- **Container runtime:** set `SOROBAN_VERIFY_DOCKER` to use another binary (for example `podman`).

Exit codes: 0 verified, 1 mismatch, 2 error. JSON schema: [docs/json-schema.md](docs/json-schema.md).

## Development

```
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

The tests mock the chain, git and the container build, except one that runs the real `git` against a
missing repository. Install from source with `cargo install --locked --path .`.
