//! `tokred update`.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tokred_core::FileSystem;
use tokred_core::release::ReleaseSource;
use tokred_update::{
    BinarySwap, Downloader, InstallMethod, detect, is_newer, self_update, upgrade_command,
};

use crate::io::Io;

/// Everything `update` needs from the outside world.
pub struct UpdateContext<'a> {
    /// The running executable (resolved, so a package-manager symlink shows its real home).
    pub exe: &'a Path,
    /// This version.
    pub current: &'a str,
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

pub fn run(io: &mut Io, ctx: &UpdateContext<'_>) -> Result<()> {
    let method = detect(&ctx.exe.to_string_lossy());
    let latest = ctx
        .releases
        .latest()
        .context("could not check for the latest release")?;
    if !is_newer(ctx.current, &latest.version) {
        writeln!(io.out(), "tokred {} is up to date.", ctx.current)?;
        return Ok(());
    }

    if method != InstallMethod::Script {
        let command = upgrade_command(method).unwrap_or("reinstall tokred");
        writeln!(
            io.out(),
            "tokred {} is available (current {}). Update with: {command}",
            latest.version,
            ctx.current
        )?;
        return Ok(());
    }

    let Some(target) = ctx.target else {
        bail!("no prebuilt tokred binary for this platform; reinstall with the method you used");
    };
    writeln!(
        io.out(),
        "Updating tokred {} -> {} ...",
        ctx.current,
        latest.version
    )?;
    let updated = self_update(
        ctx.downloader,
        ctx.fs,
        ctx.swap,
        ctx.exe,
        &latest.version,
        target,
    )?;
    writeln!(
        io.out(),
        "Updated to tokred {} ({}).",
        updated.version,
        updated.asset
    )?;
    writeln!(
        io.out(),
        "Re-applying the configuration with the new binary ..."
    )?;
    (ctx.reapply)(ctx.exe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
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
                        .map(|b| format!("{b:02x}"))
                        .collect()
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

    #[test]
    fn an_up_to_date_install_says_so() {
        let (mut io, out) = scripted(false, "");
        let exe = Path::new("/home/u/.cargo/bin/tokred");
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
            &|_| Ok(()),
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
            current: "0.1.0",
            releases,
            downloader,
            fs: &StdFileSystem,
            swap: &StdBinarySwap,
            target,
            reapply,
        }
    }

    #[test]
    fn package_managers_are_told_which_command_to_run() {
        for (path, command) in [
            (
                "/opt/homebrew/Cellar/tokred/0.1.0/bin/tokred",
                "brew upgrade tokred",
            ),
            (
                "C:\\Users\\u\\AppData\\Local\\Microsoft\\WinGet\\Packages\\x\\tokred.exe",
                "winget upgrade tokred",
            ),
            (
                "C:\\Users\\u\\scoop\\apps\\tokred\\current\\tokred.exe",
                "scoop update tokred",
            ),
            ("/home/u/.cargo/bin/tokred", "cargo install tokred --force"),
        ] {
            let (mut io, out) = scripted(false, "");
            let downloads = Downloads {
                archive: vec![],
                bad_checksum: false,
                requested: RefCell::default(),
            };
            let exe = Path::new(path);
            let ctx = ctx_with(
                &Releases(Ok("0.2.0")),
                &downloads,
                exe,
                Some(TARGET),
                &|_| Ok(()),
            );
            run(&mut io, &ctx).unwrap();
            assert!(
                out.text()
                    .contains("tokred 0.2.0 is available (current 0.1.0)"),
                "{path}"
            );
            assert!(out.text().contains(command), "{path}: {}", out.text());
            assert!(
                downloads.requested.borrow().is_empty(),
                "package managers never self-update"
            );
        }
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
            &|_| Ok(()),
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
        let ctx = ctx_with(&Releases(Ok("0.2.0")), &downloads, &exe, None, &|_| Ok(()));
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
        let exe = Path::new("/home/u/.cargo/bin/tokred");
        let ctx = ctx_with(&Releases(Err(())), &downloads, exe, Some(TARGET), &|_| {
            Ok(())
        });
        let err = run(&mut io, &ctx).unwrap_err();
        assert!(format!("{err:#}").contains("rate limited"), "{err:#}");
    }
}
