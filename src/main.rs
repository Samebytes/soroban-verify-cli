use clap::{Parser, Subcommand};
use soroban_verify::cache::{BuildCache, FileCache};
use soroban_verify::system::{DockerBuilder, GitFetcher, StellarCli};
use soroban_verify::{verify, Status, VerifyRequest};

#[derive(Parser)]
#[command(name = "soroban-verify", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Rebuild a contract from a commit and compare to the on-chain WASM hash.
    Verify {
        contract_id: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        commit: String,
        /// Toolchain image. Default: selected from the repo's soroban-sdk version.
        #[arg(long)]
        image: Option<String>,
        /// Build only this workspace package.
        #[arg(long)]
        package: Option<String>,
        /// Do not read or write the build cache.
        #[arg(long)]
        no_cache: bool,
        /// Build cache directory (default: $SOROBAN_VERIFY_CACHE or the platform cache dir).
        #[arg(long)]
        cache_dir: Option<std::path::PathBuf>,
        #[arg(long, default_value = "testnet")]
        network: String,
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
    },
}

/// Exit codes: 0 verified, 1 mismatch, 2 error.
fn main() {
    let Cmd::Verify {
        contract_id,
        repo,
        commit,
        image,
        package,
        no_cache,
        cache_dir,
        network,
        json,
    } = Cli::parse().cmd;
    let req = VerifyRequest {
        contract_id,
        repo,
        commit,
        image,
        network,
        package,
    };
    let cache = FileCache::new(cache_dir.unwrap_or_else(FileCache::default_dir));
    let cache_ref: Option<&dyn BuildCache> = if no_cache { None } else { Some(&cache) };
    match verify(&req, &StellarCli, &GitFetcher, &DockerBuilder, cache_ref) {
        Ok(r) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                println!(
                    "on-chain hash: {}\nbuilt hash:    {}",
                    r.onchain_hash, r.built_hash
                );
                println!(
                    "{}",
                    if r.status == Status::Verified {
                        "VERIFIED"
                    } else {
                        "MISMATCH"
                    }
                );
            }
            std::process::exit(if r.status == Status::Verified { 0 } else { 1 });
        }
        Err(e) => {
            if json {
                let v = serde_json::json!({"schema_version":1,"status":"error","error_kind":e.kind(),"message":e.to_string()});
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                eprintln!("error: {e}");
            }
            std::process::exit(2);
        }
    }
}
