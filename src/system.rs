//! Real implementations: `stellar` CLI, `git`, and `docker`.

use crate::{Builder, BuiltWasm, ChainClient, SourceFetcher, VerifyError};
use std::path::Path;
use std::process::Command;

pub struct StellarCli;

impl ChainClient for StellarCli {
    fn onchain_wasm_hash(&self, contract_id: &str, network: &str) -> Result<String, VerifyError> {
        let out = Command::new("stellar")
            .args([
                "contract",
                "info",
                "hash",
                "--id",
                contract_id,
                "--network",
                network,
            ])
            .output()
            .map_err(|e| VerifyError::ContractNotFound {
                network: network.into(),
                detail: format!("could not run stellar: {e}"),
            })?;
        if !out.status.success() {
            return Err(VerifyError::ContractNotFound {
                network: network.into(),
                detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(VerifyError::ContractNotFound {
                network: network.into(),
                detail: format!("unexpected hash output: {hash}"),
            });
        }
        Ok(hash)
    }
}

pub struct GitFetcher;

fn git(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output, std::io::Error> {
    let mut c = Command::new("git");
    // Disable line-ending conversion so checkouts are byte-identical on every OS.
    c.args(["-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.output()
}

impl SourceFetcher for GitFetcher {
    fn checkout(&self, repo: &str, commit: &str, dest: &Path) -> Result<(), VerifyError> {
        let d = dest.to_string_lossy();
        let out = git(&["clone", "--quiet", "--no-checkout", "--", repo, &d], None)
            .map_err(|e| VerifyError::RepoUnreachable(e.to_string()))?;
        if !out.status.success() {
            return Err(VerifyError::RepoUnreachable(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        let out = git(&["checkout", "--quiet", "--detach", commit], Some(dest))
            .map_err(|e| VerifyError::RepoUnreachable(e.to_string()))?;
        if !out.status.success() {
            return Err(VerifyError::CommitNotFound(commit.to_string()));
        }
        Ok(())
    }
}

/// Builds inside the pinned container: `stellar contract build --locked`.
pub struct DockerBuilder;

impl Builder for DockerBuilder {
    fn build(
        &self,
        src: &Path,
        image: &str,
        package: Option<&str>,
    ) -> Result<Vec<BuiltWasm>, VerifyError> {
        // The official image's entrypoint is `stellar` and its workdir is /source.
        let mount = format!("{}:/source", src.display());
        // SOROBAN_VERIFY_DOCKER overrides the container runtime binary (e.g. podman).
        let runtime = std::env::var("SOROBAN_VERIFY_DOCKER").unwrap_or_else(|_| "docker".into());
        let mut build_args = vec!["contract", "build", "--locked", "--out-dir", "/source/out"];
        if let Some(p) = package {
            build_args.extend(["--package", p]);
        }
        let out = Command::new(runtime)
            .args(["run", "--rm", "-v", &mount, "-w", "/source", image])
            .args(&build_args)
            .output()
            .map_err(|e| VerifyError::BuildFailed(format!("could not run docker: {e}")))?;
        if !out.status.success() {
            return Err(VerifyError::BuildFailed(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        let dir = src.join("out");
        let mut wasms = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "wasm") {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    wasms.push(BuiltWasm { name, path });
                }
            }
        }
        Ok(wasms)
    }
}
