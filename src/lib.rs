//! Core verification logic. All side effects (chain, git, container build) sit behind
//! traits so the flow is testable without Docker or network access.

pub mod cache;
pub mod system;
pub mod toolchain;

use cache::{cache_key, BuildCache, Hashes};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use toolchain::DEFAULT_IMAGE;

#[derive(Debug, Clone)]
pub struct VerifyRequest {
    pub contract_id: String,
    pub repo: String,
    pub commit: String,
    /// Explicit image. When `None`, it is selected from the repo's soroban-sdk version.
    pub image: Option<String>,
    pub network: String,
    /// Build only this workspace package.
    pub package: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("contract not found on {network}: {detail}")]
    ContractNotFound { network: String, detail: String },
    #[error("repository unreachable: {0}")]
    RepoUnreachable(String),
    #[error("commit {0} not found in repository")]
    CommitNotFound(String),
    #[error("build failed: {0}")]
    BuildFailed(String),
    #[error("build produced no contract wasm")]
    NoWasmProduced,
    #[error("unsupported network {0}: testnet only")]
    UnsupportedNetwork(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl VerifyError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ContractNotFound { .. } => "contract_not_found",
            Self::RepoUnreachable(_) => "repo_unreachable",
            Self::CommitNotFound(_) => "commit_not_found",
            Self::BuildFailed(_) => "build_failed",
            Self::NoWasmProduced => "no_wasm_produced",
            Self::UnsupportedNetwork(_) => "unsupported_network",
            Self::InvalidInput(_) => "invalid_input",
        }
    }
}

/// A WASM produced by a build; `name` is the file stem (the contract/package name).
#[derive(Debug, Clone)]
pub struct BuiltWasm {
    pub name: String,
    pub path: PathBuf,
}

/// Reads the on-chain WASM hash (hex, 64 chars) for a contract.
pub trait ChainClient {
    fn onchain_wasm_hash(&self, contract_id: &str, network: &str) -> Result<String, VerifyError>;
}

/// Checks a repo out at an exact commit into `dest`.
pub trait SourceFetcher {
    fn checkout(&self, repo: &str, commit: &str, dest: &Path) -> Result<(), VerifyError>;
}

/// Builds the workspace in `src` with the pinned image; returns every produced .wasm.
pub trait Builder {
    fn build(
        &self,
        src: &Path,
        image: &str,
        package: Option<&str>,
    ) -> Result<Vec<BuiltWasm>, VerifyError>;
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Verified,
    Mismatch,
}

/// Stable JSON result (schema_version 1). Documented in docs/json-schema.md.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerifyResult {
    pub schema_version: u32,
    pub status: Status,
    pub contract_id: String,
    pub network: String,
    pub repo: String,
    pub commit: String,
    pub toolchain_image: String,
    pub onchain_hash: String,
    /// Hash of the wasm that matched, or of the best candidate on a mismatch.
    pub built_hash: String,
    /// Name of the wasm that `built_hash` belongs to.
    pub wasm_name: String,
    /// Every wasm hash the build produced, by name.
    pub candidates: BTreeMap<String, String>,
    /// True when the build result came from the local cache.
    pub cached: bool,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate(req: &VerifyRequest) -> Result<(), VerifyError> {
    if req.network != "testnet" {
        return Err(VerifyError::UnsupportedNetwork(req.network.clone()));
    }
    let c = &req.commit;
    if c.len() < 7 || c.len() > 40 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(VerifyError::InvalidInput(format!(
            "commit {c} is not a hex sha"
        )));
    }
    if req.contract_id.len() != 56 || !req.contract_id.starts_with('C') {
        return Err(VerifyError::InvalidInput(
            "contract id must be a 56-char C... address".into(),
        ));
    }
    if req.repo.starts_with('-') {
        return Err(VerifyError::InvalidInput(
            "repo must not start with a dash".into(),
        ));
    }
    if let Some(p) = &req.package {
        if p.is_empty() || p.starts_with('-') {
            return Err(VerifyError::InvalidInput("invalid package name".into()));
        }
    }
    Ok(())
}

pub fn verify(
    req: &VerifyRequest,
    chain: &dyn ChainClient,
    fetcher: &dyn SourceFetcher,
    builder: &dyn Builder,
    cache: Option<&dyn BuildCache>,
) -> Result<VerifyResult, VerifyError> {
    validate(req)?;
    let onchain = chain
        .onchain_wasm_hash(&req.contract_id, &req.network)?
        .to_lowercase();

    let tmp = tempfile::tempdir().map_err(|e| VerifyError::BuildFailed(e.to_string()))?;
    let src = tmp.path().join("src");
    fetcher.checkout(&req.repo, &req.commit, &src)?;

    let image = match &req.image {
        Some(i) => i.clone(),
        None => toolchain::select_image(&src)?,
    };

    let key = cache_key(&req.repo, &req.commit, &image, req.package.as_deref());
    let (candidates, cached): (Hashes, bool) = match cache.and_then(|c| c.get(&key)) {
        Some(h) if !h.is_empty() => (h, true),
        _ => {
            let wasms = builder.build(&src, &image, req.package.as_deref())?;
            if wasms.is_empty() {
                return Err(VerifyError::NoWasmProduced);
            }
            let mut hashes = Hashes::new();
            for w in wasms {
                let bytes =
                    std::fs::read(&w.path).map_err(|e| VerifyError::BuildFailed(e.to_string()))?;
                hashes.insert(w.name, sha256_hex(&bytes));
            }
            if let Some(c) = cache {
                c.put(&key, &hashes);
            }
            (hashes, false)
        }
    };

    // A workspace may build several contracts: the deployed one is whichever matches on-chain.
    let matched = candidates.iter().find(|(_, h)| **h == onchain);
    let (status, name, hash) = match matched {
        Some((n, h)) => (Status::Verified, n.clone(), h.clone()),
        None => {
            // BTreeMap iteration is sorted, so the "best candidate" is deterministic.
            let (n, h) = candidates.iter().next().expect("non-empty");
            (Status::Mismatch, n.clone(), h.clone())
        }
    };

    Ok(VerifyResult {
        schema_version: 1,
        status,
        contract_id: req.contract_id.clone(),
        network: req.network.clone(),
        repo: req.repo.clone(),
        commit: req.commit.clone(),
        toolchain_image: image,
        onchain_hash: onchain,
        built_hash: hash,
        wasm_name: name,
        candidates,
        cached,
    })
}
