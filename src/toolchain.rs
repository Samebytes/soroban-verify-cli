//! Pinned toolchain images and selection from the repo's `soroban-sdk` version (wave: cli-3).
//!
//! The mapping is a heuristic. It picks the newest pinned image that is plausible for the SDK
//! major version found in `Cargo.lock`. A verification only means something if the image matches
//! the toolchain used at deploy time, so `--image` always wins.

use crate::VerifyError;
use std::path::Path;

/// stellar-cli 26.1.0 + Rust 1.95.0 (official image, pinned by manifest digest).
pub const IMAGE_CURRENT: &str = "docker.io/stellar/stellar-cli@sha256:daed639814911ebcafe181027fe40ab24fa6b5fe24ce0b388f95f1f38f9856ff";

/// stellar-cli 25.2.0 + Rust 1.90.0 (official image, pinned by manifest digest).
pub const IMAGE_LEGACY: &str = "docker.io/stellar/stellar-cli@sha256:d568e243c18c306af45330d05276c4ebabce05a46c2197743a4fcadbbadab927";

/// Default image when the SDK version cannot be determined.
pub const DEFAULT_IMAGE: &str = IMAGE_CURRENT;

/// Finds the `soroban-sdk` version in the text of a Cargo.lock.
pub fn sdk_version_from_lock(lock: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "name = \"soroban-sdk\"" {
            for next in lines.by_ref() {
                if let Some(v) = next.trim().strip_prefix("version = \"") {
                    return v.strip_suffix('"').map(str::to_string);
                }
                if next.trim().is_empty() {
                    break;
                }
            }
        }
    }
    None
}

/// Maps an SDK version to a pinned image.
pub fn image_for_sdk(version: &str) -> Result<&'static str, VerifyError> {
    let major: u32 = version
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .ok_or_else(|| VerifyError::InvalidInput(format!("unparseable SDK version {version}")))?;
    match major {
        23.. => Ok(IMAGE_CURRENT),
        22 => Ok(IMAGE_LEGACY),
        _ => Err(VerifyError::InvalidInput(format!(
            "soroban-sdk {version} has no pinned image; pass --image"
        ))),
    }
}

/// Chooses the image for a checked-out repo. Falls back to [`DEFAULT_IMAGE`] when there is no
/// Cargo.lock or no soroban-sdk entry.
pub fn select_image(src: &Path) -> Result<String, VerifyError> {
    let Ok(lock) = std::fs::read_to_string(src.join("Cargo.lock")) else {
        return Ok(DEFAULT_IMAGE.to_string());
    };
    match sdk_version_from_lock(&lock) {
        Some(v) => image_for_sdk(&v).map(str::to_string),
        None => Ok(DEFAULT_IMAGE.to_string()),
    }
}
