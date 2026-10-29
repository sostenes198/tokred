//! `tokred update`.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use tokred_core::FileSystem;
use tokred_core::release::ReleaseSource;
use tokred_update::{BinarySwap, Downloader, InstallMethod, is_newer, normalize, self_update};

use crate::io::Io;

/// Everything `update` needs from the outside world.
pub struct UpdateContext<'a> {
    /// The running executable (resolved, so a symlink shows its real home).
    pub exe: &'a Path,
    /// Whether the install script put it there (only then can it replace itself).
    pub method: InstallMethod,
    /// This version.
    pub current: &'a str,
    /// `--version`: the exact version to install, instead of the latest stable release.
    pub requested: Option<&'a str>,
    /// Release information.
    pub releases: &'a dyn ReleaseSource,
    /// Archive and checksum downloads.
    pub downloader: &'a dyn Downloader,
    /// File system.
    pub fs: &'a dyn FileSystem,
    /// Binary replacement.
    pub swap: &'a dyn BinarySwap,
    /// Target triple of this build, when a prebuilt binary exists for it.
    pub target: Option<&'a str>,
    /// Re-applies the configuration with the **new** binary after a self-update.
    pub reapply: &'a dyn Fn(&Path) -> Result<()>,
}

/// Where the published versions are listed (named when a requested one cannot be downloaded).
const RELEASES_PAGE: &str = "https://github.com/sostenes198/tokred/releases";

/// The version to install: the one asked for, else the latest stable release when it is newer
/// than this one (`None` when there is nothing to do, which is already said).
fn version_to_install(io: &mut Io, ctx: &UpdateContext<'_>) -> Result<Option<String>> {
    if let Some(text) = ctx.requested {
        let version = normalize(text)
            .ok_or_else(|| anyhow!("`{text}` is not a version number like 0.2.0 or 0.2.0-rc.1"))?;
        if version == ctx.current {
            writeln!(io.out(), "tokred {version} is already installed.")?;
            return Ok(None);
        }
        return Ok(Some(version));
    }
    let latest = ctx
        .releases
        .latest()
        .context("could not check for the latest release")?;
    if !is_newer(ctx.current, &latest.version) {
        writeln!(io.out(), "tokred {} is up to date.", ctx.current)?;
        return Ok(None);
    }
    Ok(Some(latest.version))
}

pub fn run(io: &mut Io, ctx: &UpdateContext<'_>) -> Result<()> {
    let Some(version) = version_to_install(io, ctx)? else {
        return Ok(());
    };
    let explicit = ctx.requested.is_some();

    if ctx.method != InstallMethod::Script {
        let message = if explicit {
            format!(
                "This copy was not put here by the install script, so it cannot replace itself: reinstall with the install script and `--version {version}` (see the README)."
            )
        } else {
            format!(
                "tokred {version} is available (current {}). This copy was not put here by the install script, so it cannot replace itself: reinstall with the install script (see the README).",
                ctx.current
            )
        };
        writeln!(io.out(), "{message}")?;
        return Ok(());
    }

    let Some(target) = ctx.target else {
        bail!("no prebuilt tokred binary for this platform; build it from source");
    };
    let verb = if is_newer(ctx.current, &version) {
        "Updating"
    } else {
        "Installing"
    };
    let message = format!("{verb} tokred {} -> {version} ...", ctx.current);
    writeln!(io.out(), "{message}")?;
    let updated = self_update(ctx.downloader, ctx.fs, ctx.swap, ctx.exe, &version, target)
        .map_err(|error| {
            if explicit {
                anyhow!(
                    "could not install tokred {version}: {error}\nIs it a published version? See {RELEASES_PAGE}"
                )
            } else {
                anyhow::Error::from(error)
            }
        })?;
    let message = format!("Updated to tokred {} ({}).", updated.version, updated.asset);
    writeln!(io.out(), "{message}")?;
    let note = "Re-applying the configuration with the new binary ...";
    writeln!(io.out(), "{note}")?;
    (ctx.reapply)(ctx.exe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fmt::Write as _;
    use std::io::Write;

    use sha2::{Digest, Sha256};
    use tokred_core::pricing::SourceError;
    use tokred_core::release::Release;
    use tokred_install::StdFileSystem;
    use tokred_update::StdBinarySwap;

    use super::*;
    use crate::io::testing::scripted;

    struct Releases(Result<&'static str, ()>);
    impl ReleaseSource for Releases {
        fn latest(&self) -> Result<Release, SourceError> {
            match self.0 {
                Ok(version) => Ok(Release {
                    version: version.to_owned(),
                    url: String::new(),
                }),
                Err(()) => Err(SourceError::Invalid("rate limited".to_owned())),
            }
        }
    }

    /// Serves a tar.gz with a fake `tokred` binary and its checksum.
    struct Downloads {
        archive: Vec<u8>,
        bad_checksum: bool,
        requested: RefCell<Vec<String>>,
    }

    impl Downloader for Downloads {
        fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
            self.requested.borrow_mut().push(url.to_owned());
            if url.ends_with(".sha256") {
                let digest = if self.bad_checksum {
                    "0".repeat(64)
                } else {
                    Sha256::digest(&self.archive)
                        .iter()
                        .fold(String::new(), |mut hex, b| {
                            let _ = write!(hex, "{b:02x}");
                            hex
                        })
                };
                return Ok(format!("{digest}  asset\n").into_bytes());
            }
            Ok(self.archive.clone())
        }
    }

    fn tar_gz(binary: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(binary.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                "tokred-x86_64-unknown-linux-musl/tokred",
                binary,
            )
            .unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    const TARGET: &str = "x86_64-unknown-linux-musl";

    #[allow(clippy::unnecessary_wraps)] // the signature is dictated by `UpdateContext::reapply`
    fn no_reapply(_: &Path) -> Result<()> {
        Ok(())
    }

    #[test]
    fn the_no_op_reapply_helper_succeeds() {
        no_reapply(Path::new("x")).unwrap();
    }

    #[test]
    fn an_up_to_date_install_says_so() {
        let (mut io, out) = scripted(false, "");
        let exe = Path::new("/home/u/.tokred/bin/tokred");
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let ctx = ctx_with(
            &Releases(Ok("0.1.0")),
            &downloads,
            exe,
            Some(TARGET),
            &no_reapply,
        );
        run(&mut io, &ctx).unwrap();
        assert_eq!(out.text(), "tokred 0.1.0 is up to date.\n");
    }

    fn ctx_with<'a>(
        releases: &'a dyn ReleaseSource,
        downloader: &'a dyn Downloader,
        exe: &'a Path,
        target: Option<&'a str>,
        reapply: &'a dyn Fn(&Path) -> Result<()>,
    ) -> UpdateContext<'a> {
        UpdateContext {
            exe,
            method: InstallMethod::Script,
            current: "0.1.0",
            requested: None,
            releases,
            downloader,
            fs: &StdFileSystem,
            swap: &StdBinarySwap,
            target,
            reapply,
        }
    }

    #[test]
    fn a_binary_the_script_did_not_install_is_told_to_reinstall() {
        let (mut io, out) = scripted(false, "");
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let exe = Path::new("/work/tokred/target/debug/tokred");
        let mut ctx = ctx_with(
            &Releases(Ok("0.2.0")),
            &downloads,
            exe,
            Some(TARGET),
            &no_reapply,
        );
        ctx.method = InstallMethod::Other;
        run(&mut io, &ctx).unwrap();
        let text = out.text();
        assert!(
            text.contains("tokred 0.2.0 is available (current 0.1.0)"),
            "{text}"
        );
        assert!(text.contains("reinstall with the install script"), "{text}");
        assert!(
            downloads.requested.borrow().is_empty(),
            "only the script's own copy is replaced"
        );
    }

    #[test]
    fn a_script_install_replaces_itself_and_re_applies_the_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tokred");
        std::fs::write(&exe, "old binary").unwrap();
        let downloads = Downloads {
            archive: tar_gz(b"new binary"),
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let reapplied = Cell::new(false);
        let reapply = |path: &Path| -> Result<()> {
            assert_eq!(path, exe.as_path());
            reapplied.set(true);
            Ok(())
        };

        let (mut io, out) = scripted(false, "");
        let ctx = ctx_with(
            &Releases(Ok("0.2.0")),
            &downloads,
            &exe,
            Some(TARGET),
            &reapply,
        );
        run(&mut io, &ctx).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        assert!(reapplied.get());
        let text = out.text();
        assert!(text.contains("Updating tokred 0.1.0 -> 0.2.0"), "{text}");
        assert!(text.contains("Updated to tokred 0.2.0"));
        assert!(text.contains("Re-applying the configuration"));
        assert!(downloads.requested.borrow()[0].contains("v0.2.0"));
    }

    fn with_request<'a>(mut ctx: UpdateContext<'a>, version: &'a str) -> UpdateContext<'a> {
        ctx.requested = Some(version);
        ctx
    }

    #[test]
    fn a_requested_version_is_installed_even_when_it_is_a_pre_release_or_older() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tokred");
        std::fs::write(&exe, "old binary").unwrap();
        let downloads = Downloads {
            archive: tar_gz(b"rc binary"),
            bad_checksum: false,
            requested: RefCell::default(),
        };
        // The latest stable release is not asked about: a failing source changes nothing.
        let releases = Releases(Err(()));
        let (mut io, out) = scripted(false, "");
        let ctx = with_request(
            ctx_with(&releases, &downloads, &exe, Some(TARGET), &no_reapply),
            "v0.2.0-rc.3",
        );
        run(&mut io, &ctx).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"rc binary");
        let text = out.text();
        assert!(
            text.contains("Updating tokred 0.1.0 -> 0.2.0-rc.3"),
            "{text}"
        );
        assert!(downloads.requested.borrow()[0].contains("v0.2.0-rc.3"));

        // An older one is a downgrade, and says "Installing".
        let (mut io, out) = scripted(false, "");
        let ctx = with_request(
            ctx_with(&releases, &downloads, &exe, Some(TARGET), &no_reapply),
            "0.0.9",
        );
        run(&mut io, &ctx).unwrap();
        assert!(
            out.text().contains("Installing tokred 0.1.0 -> 0.0.9"),
            "{}",
            out.text()
        );
    }

    #[test]
    fn the_version_already_installed_is_not_downloaded_again() {
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let exe = Path::new("/home/u/.tokred/bin/tokred");
        let (mut io, out) = scripted(false, "");
        let ctx = with_request(
            ctx_with(
                &Releases(Err(())),
                &downloads,
                exe,
                Some(TARGET),
                &no_reapply,
            ),
            "v0.1.0",
        );
        run(&mut io, &ctx).unwrap();
        assert_eq!(out.text(), "tokred 0.1.0 is already installed.\n");
        assert!(downloads.requested.borrow().is_empty());
    }

    #[test]
    fn a_text_that_is_not_a_version_is_refused_before_anything_is_downloaded() {
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let exe = Path::new("/home/u/.tokred/bin/tokred");
        for bad in ["latest", "../../evil", "1.0"] {
            let (mut io, _) = scripted(false, "");
            let ctx = with_request(
                ctx_with(
                    &Releases(Err(())),
                    &downloads,
                    exe,
                    Some(TARGET),
                    &no_reapply,
                ),
                bad,
            );
            let err = run(&mut io, &ctx).unwrap_err();
            assert!(
                err.to_string().contains("is not a version number"),
                "{bad}: {err}"
            );
        }
        assert!(downloads.requested.borrow().is_empty());
    }

    #[test]
    fn a_requested_version_that_is_not_published_names_where_to_look() {
        struct Missing;
        impl Downloader for Missing {
            fn download(&self, _: &str) -> Result<Vec<u8>, SourceError> {
                Err(SourceError::Invalid("http status: 404".to_owned()))
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tokred");
        std::fs::write(&exe, "old binary").unwrap();
        let (mut io, _) = scripted(false, "");
        let ctx = with_request(
            ctx_with(
                &Releases(Err(())),
                &Missing,
                &exe,
                Some(TARGET),
                &no_reapply,
            ),
            "9.9.9",
        );
        let err = run(&mut io, &ctx).unwrap_err().to_string();
        assert!(err.contains("could not install tokred 9.9.9"), "{err}");
        assert!(err.contains("404") && err.contains("/releases"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
    }

    #[test]
    fn a_copy_the_script_did_not_install_is_told_how_to_ask_for_that_version() {
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let exe = Path::new("/work/tokred/target/debug/tokred");
        let (mut io, out) = scripted(false, "");
        let mut ctx = with_request(
            ctx_with(
                &Releases(Err(())),
                &downloads,
                exe,
                Some(TARGET),
                &no_reapply,
            ),
            "0.2.0-rc.3",
        );
        ctx.method = InstallMethod::Other;
        run(&mut io, &ctx).unwrap();
        assert!(
            out.text().contains("`--version 0.2.0-rc.3`"),
            "{}",
            out.text()
        );
        assert!(downloads.requested.borrow().is_empty());
    }

    #[test]
    fn a_bad_checksum_aborts_and_leaves_the_binary_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tokred");
        std::fs::write(&exe, "old binary").unwrap();
        let downloads = Downloads {
            archive: tar_gz(b"new binary"),
            bad_checksum: true,
            requested: RefCell::default(),
        };
        let (mut io, _) = scripted(false, "");
        let ctx = ctx_with(
            &Releases(Ok("0.2.0")),
            &downloads,
            &exe,
            Some(TARGET),
            &no_reapply,
        );
        let err = run(&mut io, &ctx).unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old binary");
    }

    #[test]
    fn a_failing_reapply_is_an_error_and_a_missing_platform_build_is_explained() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("tokred");
        std::fs::write(&exe, "old").unwrap();
        let downloads = Downloads {
            archive: tar_gz(b"new"),
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let (mut io, _) = scripted(false, "");
        let ctx = ctx_with(
            &Releases(Ok("0.2.0")),
            &downloads,
            &exe,
            Some(TARGET),
            &|_| bail!("configure failed"),
        );
        assert!(
            run(&mut io, &ctx)
                .unwrap_err()
                .to_string()
                .contains("configure failed")
        );

        let (mut io, _) = scripted(false, "");
        let ctx = ctx_with(&Releases(Ok("0.2.0")), &downloads, &exe, None, &no_reapply);
        assert!(
            run(&mut io, &ctx)
                .unwrap_err()
                .to_string()
                .contains("no prebuilt tokred binary")
        );
    }

    #[test]
    fn an_unreachable_release_api_is_an_error() {
        let (mut io, _) = scripted(false, "");
        let downloads = Downloads {
            archive: vec![],
            bad_checksum: false,
            requested: RefCell::default(),
        };
        let exe = Path::new("/home/u/.tokred/bin/tokred");
        let ctx = ctx_with(
            &Releases(Err(())),
            &downloads,
            exe,
            Some(TARGET),
            &no_reapply,
        );
        let err = run(&mut io, &ctx).unwrap_err();
        assert!(format!("{err:#}").contains("rate limited"), "{err:#}");
    }
}
