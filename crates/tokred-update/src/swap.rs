//! Self-update for script installs: download, verify the checksum, extract, swap the binary.
//!
//! Release assets are produced by `dist`: `tokred-<target-triple>.tar.gz` on Unix and
//! `tokred-<target-triple>.zip` on Windows, each next to a `<asset>.sha256` file that holds
//! `<hex>  <name>`. The archive nests the binary in a directory next to the README and licences,
//! so the binary is found by its file name.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokred_core::error::FsError;
use tokred_core::ports::FileSystem;
use tokred_core::pricing::SourceError;

/// Where release assets live; the tag is `v<version>`.
const RELEASE_DOWNLOAD_BASE: &str = "https://github.com/sostenes198/tokred/releases/download";

/// Upper bound for the extracted binary (a decompression bomb guard; real binaries are ~10 MB).
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

/// Downloads a URL into memory (network adapter: [`crate::http::UreqDownloader`]).
pub trait Downloader {
    /// Returns the body of `url`.
    fn download(&self, url: &str) -> Result<Vec<u8>, SourceError>;
}

/// The two file operations the file system port does not offer.
pub trait BinarySwap {
    /// Renames `from` to `to`, replacing `to` when the platform allows it.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError>;
    /// Marks the file as executable (a no-op where the concept does not exist).
    fn set_executable(&self, path: &Path) -> Result<(), FsError>;
}

/// [`BinarySwap`] on the real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdBinarySwap;

impl BinarySwap for StdBinarySwap {
    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        std::fs::rename(from, to).map_err(|err| FsError::new(to, err))
    }

    #[cfg(unix)]
    fn set_executable(&self, path: &Path) -> Result<(), FsError> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|err| FsError::new(path, err))
    }

    #[cfg(not(unix))]
    fn set_executable(&self, _path: &Path) -> Result<(), FsError> {
        Ok(())
    }
}

/// Why a self-update did not happen. Nothing is modified on any of these errors except
/// [`UpdateError::Fs`], and even then the previous binary is restored when possible.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// No release asset exists for this platform.
    #[error("no prebuilt tokred binary for this platform; reinstall with the method you used")]
    UnsupportedPlatform,
    /// The asset or its checksum could not be downloaded.
    #[error("download failed: {0}")]
    Download(#[from] SourceError),
    /// The `.sha256` file does not contain a SHA-256 digest.
    #[error("checksum file is malformed")]
    MalformedChecksum,
    /// The download does not match its checksum.
    #[error("checksum mismatch (expected {expected}, got {actual}); the update was aborted")]
    ChecksumMismatch {
        /// Digest published with the release.
        expected: String,
        /// Digest of what was downloaded.
        actual: String,
    },
    /// The archive could not be read.
    #[error("could not read the archive: {0}")]
    Archive(#[from] std::io::Error),
    /// The zip archive could not be read.
    #[error("could not read the zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The archive does not contain the tokred binary.
    #[error("the archive does not contain `{0}`")]
    BinaryMissing(String),
    /// Replacing the binary failed.
    #[error(transparent)]
    Fs(#[from] FsError),
}

/// What a successful [`self_update`] installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdated {
    /// The version now installed (without a leading `v`).
    pub version: String,
    /// The release asset it came from.
    pub asset: String,
}

/// The target triple of the running platform, as used in `dist` asset names; `None` when no
/// asset is published for it. Linux always maps to the musl build (spec section 14).
#[must_use]
pub fn current_target() -> Option<String> {
    target_triple(std::env::consts::ARCH, std::env::consts::OS)
}

/// Maps `std::env::consts::{ARCH, OS}` to a target triple.
#[must_use]
pub fn target_triple(arch: &str, os: &str) -> Option<String> {
    if !matches!(arch, "x86_64" | "aarch64") {
        return None;
    }
    let suffix = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        "windows" => "pc-windows-msvc",
        _ => return None,
    };
    Some(format!("{arch}-{suffix}"))
}

/// Name of the release asset for `target`: `.zip` for Windows, `.tar.gz` otherwise.
#[must_use]
pub fn asset_name(target: &str) -> String {
    let extension = if is_windows(target) { "zip" } else { "tar.gz" };
    format!("tokred-{target}.{extension}")
}

/// Download URL of `asset` in release `version` (tag `v<version>`).
#[must_use]
pub fn release_asset_url(version: &str, asset: &str) -> String {
    let bare = version.trim().trim_start_matches(['v', 'V']);
    format!("{RELEASE_DOWNLOAD_BASE}/v{bare}/{asset}")
}

/// Path the previous binary is parked at while replacing a running Windows executable
/// (`tokred.exe` becomes `tokred.exe.old`).
#[must_use]
pub fn old_binary_path(exe_path: &Path) -> PathBuf {
    with_suffix(exe_path, ".old")
}

/// Deletes the `.old` binary left by a Windows self-update; returns whether one was removed.
/// Failures are ignored (the file may still be locked; the next run tries again).
pub fn cleanup_old_binary(fs: &dyn FileSystem, exe_path: &Path) -> bool {
    fs.remove_file(&old_binary_path(exe_path)).unwrap_or(false)
}

/// Replaces the executable at `exe_path` with release `version` for `target`.
///
/// Downloads the asset and its `.sha256` file and verifies the digest *before* touching anything:
/// on a mismatch (or any earlier error) the installed binary is left exactly as it was. Then the
/// `tokred` binary is extracted in memory and swapped in: on Unix by an atomic rename over the
/// running file, on Windows by first renaming the running `.exe` to `.old` (see
/// [`cleanup_old_binary`]), with a rollback if the second rename fails.
///
/// The caller decides whether an update is wanted (mode, install method, `is_newer`) and
/// reapplies the configuration afterwards.
pub fn self_update(
    downloader: &dyn Downloader,
    fs: &dyn FileSystem,
    swap: &dyn BinarySwap,
    exe_path: &Path,
    version: &str,
    target: &str,
) -> Result<SelfUpdated, UpdateError> {
    let windows = is_windows(target);
    let asset = asset_name(target);
    let url = release_asset_url(version, &asset);
    let archive = downloader.download(&url)?;
    let sidecar = downloader.download(&format!("{url}.sha256"))?;

    let expected =
        parse_checksum(&String::from_utf8_lossy(&sidecar)).ok_or(UpdateError::MalformedChecksum)?;
    let actual = sha256_hex(&archive);
    if actual != expected {
        return Err(UpdateError::ChecksumMismatch { expected, actual });
    }

    let binary_name = if windows { "tokred.exe" } else { "tokred" };
    let binary = if windows {
        extract_from_zip(&archive, binary_name)?
    } else {
        extract_from_tar_gz(&archive, binary_name)?
    };

    let staged = with_suffix(exe_path, ".new");
    let installed = install_binary(fs, swap, exe_path, &staged, &binary, windows);
    if installed.is_err() {
        // Best effort: do not leave the staged file behind.
        let _ = fs.remove_file(&staged);
    }
    installed?;
    Ok(SelfUpdated {
        version: version.trim().trim_start_matches(['v', 'V']).to_owned(),
        asset,
    })
}

fn install_binary(
    fs: &dyn FileSystem,
    swap: &dyn BinarySwap,
    exe_path: &Path,
    staged: &Path,
    binary: &[u8],
    windows: bool,
) -> Result<(), FsError> {
    fs.write_atomic(staged, binary)?;
    swap.set_executable(staged)?;
    if !windows {
        return swap.rename(staged, exe_path);
    }
    // A running .exe can be renamed but not overwritten.
    let old = old_binary_path(exe_path);
    fs.remove_file(&old)?;
    swap.rename(exe_path, &old)?;
    swap.rename(staged, exe_path).inspect_err(|_| {
        // Put the running binary back so the installation is never left without tokred.
        let _ = swap.rename(&old, exe_path);
    })
}

fn is_windows(target: &str) -> bool {
    target.contains("windows")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            // Writing to a `String` cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The digest in a `<hex>  <name>` checksum file (lowercased), if it is a SHA-256 digest.
fn parse_checksum(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?.trim_start_matches('*');
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

/// The last component of an archive path, with either separator.
fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>, std::io::Error> {
    let mut buffer = Vec::new();
    reader.take(MAX_BINARY_BYTES).read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn extract_from_tar_gz(archive: &[u8], name: &str) -> Result<Vec<u8>, UpdateError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let entry = entry?;
        let is_binary = entry.header().entry_type().is_file()
            && entry.path()?.file_name().is_some_and(|file| file == name);
        if is_binary {
            return Ok(read_limited(entry)?);
        }
    }
    Err(UpdateError::BinaryMissing(name.to_owned()))
}

fn extract_from_zip(archive: &[u8], name: &str) -> Result<Vec<u8>, UpdateError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
    for index in 0..zip.len() {
        let file = zip.by_index(index)?;
        if file.is_file() && base_name(&file.name()?) == name {
            return Ok(read_limited(file)?);
        }
    }
    Err(UpdateError::BinaryMissing(name.to_owned()))
}

#[cfg(test)]
mod tests;
