use soroban_verify::cache::{BuildCache, FileCache, Hashes};
use soroban_verify::system::GitFetcher;
use soroban_verify::toolchain::{self, IMAGE_CURRENT, IMAGE_LEGACY};
use soroban_verify::*;
use std::cell::{Cell, RefCell};
use std::path::Path;

const CID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const COMMIT: &str = "abcdef1234567";

struct Chain(Result<String, ()>);
impl ChainClient for Chain {
    fn onchain_wasm_hash(&self, _: &str, n: &str) -> Result<String, VerifyError> {
        self.0.clone().map_err(|_| VerifyError::ContractNotFound {
            network: n.into(),
            detail: "missing".into(),
        })
    }
}

/// Fetcher that creates an empty source tree, optionally with a Cargo.lock.
struct OkFetch(Option<&'static str>);
impl SourceFetcher for OkFetch {
    fn checkout(&self, _: &str, _: &str, d: &Path) -> Result<(), VerifyError> {
        std::fs::create_dir_all(d).unwrap();
        if let Some(lock) = self.0 {
            std::fs::write(d.join("Cargo.lock"), lock).unwrap();
        }
        Ok(())
    }
}

/// Builder that "produces" the given named wasms and records how it was called.
struct FakeBuild {
    wasms: Vec<(&'static str, &'static [u8])>,
    calls: Cell<u32>,
    last_image: RefCell<String>,
    last_package: RefCell<Option<String>>,
}
impl FakeBuild {
    fn new(wasms: Vec<(&'static str, &'static [u8])>) -> Self {
        Self {
            wasms,
            calls: Cell::new(0),
            last_image: RefCell::new(String::new()),
            last_package: RefCell::new(None),
        }
    }
    fn one(bytes: &'static [u8]) -> Self {
        Self::new(vec![("c", bytes)])
    }
}
impl Builder for FakeBuild {
    fn build(
        &self,
        src: &Path,
        image: &str,
        package: Option<&str>,
    ) -> Result<Vec<BuiltWasm>, VerifyError> {
        self.calls.set(self.calls.get() + 1);
        *self.last_image.borrow_mut() = image.to_string();
        *self.last_package.borrow_mut() = package.map(str::to_string);
        Ok(self
            .wasms
            .iter()
            .map(|(name, bytes)| {
                let path = src.join(format!("{name}.wasm"));
                std::fs::write(&path, bytes).unwrap();
                BuiltWasm {
                    name: name.to_string(),
                    path,
                }
            })
            .collect())
    }
}

#[derive(Default)]
struct MemCache(RefCell<std::collections::HashMap<String, Hashes>>);
impl BuildCache for MemCache {
    fn get(&self, k: &str) -> Option<Hashes> {
        self.0.borrow().get(k).cloned()
    }
    fn put(&self, k: &str, h: &Hashes) {
        self.0.borrow_mut().insert(k.to_string(), h.clone());
    }
}

fn req(repo: &str) -> VerifyRequest {
    VerifyRequest {
        contract_id: CID.into(),
        repo: repo.into(),
        commit: COMMIT.into(),
        image: Some("img".into()),
        network: "testnet".into(),
        package: None,
    }
}

fn run(
    r: &VerifyRequest,
    chain: &Chain,
    fetch: &dyn SourceFetcher,
    b: &FakeBuild,
    cache: Option<&dyn BuildCache>,
) -> Result<VerifyResult, VerifyError> {
    verify(r, chain, fetch, b, cache)
}

// ---- MVP behaviours -------------------------------------------------------------------

#[test]
fn matching_build_is_verified() {
    let h = sha256_hex(b"wasm-a");
    let r = run(
        &req("r"),
        &Chain(Ok(h)),
        &OkFetch(None),
        &FakeBuild::one(b"wasm-a"),
        None,
    )
    .unwrap();
    assert_eq!(r.status, Status::Verified);
    assert_eq!(r.onchain_hash, r.built_hash);
    assert_eq!(r.wasm_name, "c");
}

#[test]
fn mismatched_source_is_reported_with_both_hashes() {
    let h = sha256_hex(b"wasm-a");
    let r = run(
        &req("r"),
        &Chain(Ok(h.clone())),
        &OkFetch(None),
        &FakeBuild::one(b"tampered"),
        None,
    )
    .unwrap();
    assert_eq!(r.status, Status::Mismatch);
    assert_eq!(r.onchain_hash, h);
    assert_eq!(r.built_hash, sha256_hex(b"tampered"));
}

#[test]
fn unreachable_repo_errors() {
    let e = run(
        &req("/definitely/not/a/repo"),
        &Chain(Ok(sha256_hex(b"x"))),
        &GitFetcher,
        &FakeBuild::one(b"x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind(), "repo_unreachable");
}

#[test]
fn contract_not_found_errors() {
    let e = run(
        &req("r"),
        &Chain(Err(())),
        &OkFetch(None),
        &FakeBuild::one(b"x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind(), "contract_not_found");
}

#[test]
fn mainnet_rejected_and_json_schema_stable() {
    let mut r = req("r");
    r.network = "mainnet".into();
    let e = run(
        &r,
        &Chain(Ok("".into())),
        &OkFetch(None),
        &FakeBuild::one(b"x"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind(), "unsupported_network");

    let res = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(None),
        &FakeBuild::one(b"w"),
        None,
    )
    .unwrap();
    let v = serde_json::to_value(&res).unwrap();
    for k in [
        "schema_version",
        "status",
        "contract_id",
        "network",
        "repo",
        "commit",
        "toolchain_image",
        "onchain_hash",
        "built_hash",
        "wasm_name",
        "candidates",
        "cached",
    ] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["status"], "verified");
}

// ---- cli-1: multi-contract workspaces ---------------------------------------------------

#[test]
fn workspace_matches_the_deployed_contract_among_several() {
    let b = FakeBuild::new(vec![("alpha", b"A"), ("beta", b"B"), ("gamma", b"G")]);
    let r = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"B"))),
        &OkFetch(None),
        &b,
        None,
    )
    .unwrap();
    assert_eq!(r.status, Status::Verified);
    assert_eq!(r.wasm_name, "beta");
    assert_eq!(r.candidates.len(), 3);
}

#[test]
fn workspace_mismatch_lists_every_candidate() {
    let b = FakeBuild::new(vec![("beta", b"B"), ("alpha", b"A")]);
    let r = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"other"))),
        &OkFetch(None),
        &b,
        None,
    )
    .unwrap();
    assert_eq!(r.status, Status::Mismatch);
    // Deterministic best candidate: first by name.
    assert_eq!(r.wasm_name, "alpha");
    assert_eq!(r.candidates["beta"], sha256_hex(b"B"));
}

#[test]
fn package_flag_is_forwarded_to_the_builder() {
    let b = FakeBuild::one(b"x");
    let mut r = req("r");
    r.package = Some("my-pkg".into());
    run(&r, &Chain(Ok(sha256_hex(b"x"))), &OkFetch(None), &b, None).unwrap();
    assert_eq!(b.last_package.borrow().as_deref(), Some("my-pkg"));
}

#[test]
fn empty_build_is_an_error() {
    let b = FakeBuild::new(vec![]);
    let e = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"x"))),
        &OkFetch(None),
        &b,
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind(), "no_wasm_produced");
}

// ---- cli-2: build cache -----------------------------------------------------------------

#[test]
fn second_run_hits_the_cache_and_skips_the_build() {
    let cache = MemCache::default();
    let b = FakeBuild::one(b"w");
    let chain = Chain(Ok(sha256_hex(b"w")));
    let first = run(&req("r"), &chain, &OkFetch(None), &b, Some(&cache)).unwrap();
    let second = run(&req("r"), &chain, &OkFetch(None), &b, Some(&cache)).unwrap();
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(b.calls.get(), 1, "build must run only once");
    assert_eq!(second.status, Status::Verified);
}

#[test]
fn cache_is_keyed_by_commit_and_toolchain() {
    let cache = MemCache::default();
    let b = FakeBuild::one(b"w");
    let chain = Chain(Ok(sha256_hex(b"w")));
    run(&req("r"), &chain, &OkFetch(None), &b, Some(&cache)).unwrap();
    let mut other_image = req("r");
    other_image.image = Some("other-img".into());
    run(&other_image, &chain, &OkFetch(None), &b, Some(&cache)).unwrap();
    let mut other_commit = req("r");
    other_commit.commit = "1234567abcdef".into();
    run(&other_commit, &chain, &OkFetch(None), &b, Some(&cache)).unwrap();
    assert_eq!(b.calls.get(), 3, "each distinct key builds once");
}

#[test]
fn a_cached_mismatch_still_detects_a_different_onchain_hash() {
    let cache = MemCache::default();
    let b = FakeBuild::one(b"w");
    run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(None),
        &b,
        Some(&cache),
    )
    .unwrap();
    let r = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"z"))),
        &OkFetch(None),
        &b,
        Some(&cache),
    )
    .unwrap();
    assert!(r.cached);
    assert_eq!(r.status, Status::Mismatch);
}

#[test]
fn file_cache_round_trips_and_survives_missing_dir() {
    let dir = tempfile::tempdir().unwrap();
    let c = FileCache::new(dir.path().join("nested").join("cache"));
    let mut h = Hashes::new();
    h.insert("a".into(), "1".repeat(64));
    assert!(c.get("k").is_none());
    c.put("k", &h);
    assert_eq!(c.get("k"), Some(h));
}

// ---- cli-3: image selected from the repo's SDK version ------------------------------------

const LOCK_28: &str = "[[package]]\nname = \"soroban-sdk\"\nversion = \"28.0.0\"\n";
const LOCK_22: &str = "[[package]]\nname = \"soroban-sdk\"\nversion = \"22.0.8\"\n";
const LOCK_21: &str = "[[package]]\nname = \"soroban-sdk\"\nversion = \"21.7.0\"\n";

#[test]
fn newer_sdk_selects_the_current_image() {
    let b = FakeBuild::one(b"w");
    let mut r = req("r");
    r.image = None;
    let res = run(
        &r,
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(Some(LOCK_28)),
        &b,
        None,
    )
    .unwrap();
    assert_eq!(res.toolchain_image, IMAGE_CURRENT);
    assert_eq!(*b.last_image.borrow(), IMAGE_CURRENT);
}

#[test]
fn sdk_22_selects_the_second_pinned_image() {
    let b = FakeBuild::one(b"w");
    let mut r = req("r");
    r.image = None;
    let res = run(
        &r,
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(Some(LOCK_22)),
        &b,
        None,
    )
    .unwrap();
    assert_eq!(res.toolchain_image, IMAGE_LEGACY);
    assert_ne!(IMAGE_LEGACY, IMAGE_CURRENT);
}

#[test]
fn unsupported_sdk_requires_an_explicit_image() {
    let mut r = req("r");
    r.image = None;
    let e = run(
        &r,
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(Some(LOCK_21)),
        &FakeBuild::one(b"w"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.kind(), "invalid_input");
}

#[test]
fn explicit_image_always_wins_and_missing_lock_uses_default() {
    let b = FakeBuild::one(b"w");
    let res = run(
        &req("r"),
        &Chain(Ok(sha256_hex(b"w"))),
        &OkFetch(Some(LOCK_22)),
        &b,
        None,
    )
    .unwrap();
    assert_eq!(res.toolchain_image, "img");

    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        toolchain::select_image(dir.path()).unwrap(),
        toolchain::DEFAULT_IMAGE
    );
}

#[test]
fn sdk_version_parser_finds_the_sdk_entry() {
    let lock = "[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"soroban-sdk\"\nversion = \"23.1.2\"\n";
    assert_eq!(
        toolchain::sdk_version_from_lock(lock).as_deref(),
        Some("23.1.2")
    );
    assert_eq!(toolchain::sdk_version_from_lock("nothing"), None);
}
