# `--json` output schema (schema_version 1)

New fields may be added within a schema version; existing fields never change meaning or type.

Success (exit 0 verified, exit 1 mismatch):

| field | type | notes |
|---|---|---|
| schema_version | int | currently `1` |
| status | `"verified"` or `"mismatch"` | |
| contract_id | string | |
| network | string | `testnet` only |
| repo, commit | string | as supplied |
| toolchain_image | string | image used for the rebuild (explicit, or selected from the SDK version) |
| onchain_hash | hex string | SHA-256 of the deployed WASM |
| built_hash | hex string | SHA-256 of the matching WASM; on a mismatch, of the first candidate by name |
| wasm_name | string | name of the WASM that `built_hash` belongs to |
| candidates | object | every built WASM: name to SHA-256 (several in a workspace) |
| cached | bool | true when the build result came from the local cache |

Error (exit 2): `{schema_version, status:"error", error_kind, message}` where `error_kind` is one of
`contract_not_found`, `repo_unreachable`, `commit_not_found`, `build_failed`, `no_wasm_produced`,
`unsupported_network`, `invalid_input`.
