use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

use super::*;

const LINUX: &str = "x86_64-unknown-linux-musl";
const WINDOWS: &str = "x86_64-pc-windows-msvc";
const NEW_BINARY: &[u8] = b"new tokred binary";

// ---- fixtures -------------------------------------------------------------------------------

fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn tar_gz_with_directory_named_tokred() -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_mode(0o755);
    dir.set_cksum();
    builder
        .append_data(&mut dir, "pkg/tokred", std::io::empty())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer
        .add_directory("tokred-pkg/tokred.exe/", options)
        .unwrap();
    for (path, data) in files {
        writer.start_file(*path, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn sidecar(archive: &[u8], asset: &str) -> Vec<u8> {
    format!("{}  {asset}\n", sha256_hex(archive)).into_bytes()
}

// ---- fakes ----------------------------------------------------------------------------------

#[derive(Default)]
struct FakeDownloader {
    bodies: HashMap<String, Vec<u8>>,
    requested: RefCell<Vec<String>>,
}

impl Downloader for FakeDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        self.requested.borrow_mut().push(url.to_owned());
        self.bodies
            .get(url)
            .cloned()
            .ok_or_else(|| SourceError::Invalid(format!("404 {url}")))
    }
}

/// The file system and the swap operations over one in-memory map, so rollbacks are observable.
#[derive(Default)]
struct World {
    files: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
    executable: RefCell<Vec<PathBuf>>,
    fail_write: Cell<bool>,
    fail_chmod: Cell<bool>,
    fail_remove: Cell<bool>,
    fail_rename_to: RefCell<Option<PathBuf>>,
}

fn io_failure(path: &Path) -> FsError {
    FsError::new(path, std::io::Error::other("injected failure"))
}

impl World {
    fn with_exe(path: &str, content: &[u8]) -> Self {
        let world = Self::default();
        world
            .files
            .borrow_mut()
            .insert(PathBuf::from(path), content.to_vec());
        world
    }

    fn content(&self, path: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(Path::new(path)).cloned()
    }

    fn paths(&self) -> Vec<String> {
        self.files
            .borrow()
            .keys()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }
}

impl FileSystem for World {
    fn read_to_string(&self, _path: &Path) -> Result<Option<String>, FsError> {
        Ok(None)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), FsError> {
        if self.fail_write.get() {
            return Err(io_failure(path));
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<bool, FsError> {
        if self.fail_remove.get() {
            return Err(io_failure(path));
        }
        Ok(self.files.borrow_mut().remove(path).is_some())
    }

    fn remove_dir_all(&self, _path: &Path) -> Result<bool, FsError> {
        Ok(false)
    }

    fn create_dir_all(&self, _path: &Path) -> Result<(), FsError> {
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn is_dir(&self, _path: &Path) -> bool {
        false
    }
}

impl BinarySwap for World {
    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        if self.fail_rename_to.borrow().as_deref() == Some(to) {
            return Err(io_failure(to));
        }
        let content = self
            .files
            .borrow_mut()
            .remove(from)
            .ok_or_else(|| io_failure(from))?;
        self.files.borrow_mut().insert(to.to_path_buf(), content);
        Ok(())
    }

    fn set_executable(&self, path: &Path) -> Result<(), FsError> {
        if self.fail_chmod.get() {
            return Err(io_failure(path));
        }
        self.executable.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

fn downloader_for(target: &str, version: &str, archive: &[u8], checksum: &[u8]) -> FakeDownloader {
    let asset = asset_name(target);
    let url = release_asset_url(version, &asset);
    let mut downloader = FakeDownloader::default();
    downloader
        .bodies
        .insert(format!("{url}.sha256"), checksum.to_vec());
    downloader.bodies.insert(url, archive.to_vec());
    downloader
}

fn linux_archive() -> Vec<u8> {
    tar_gz(&[
        ("tokred-x86_64-unknown-linux-musl/README.md", b"readme"),
        ("tokred-x86_64-unknown-linux-musl/tokred", NEW_BINARY),
    ])
}

fn windows_archive() -> Vec<u8> {
    zip_of(&[
        ("tokred-x86_64-pc-windows-msvc/README.md", b"readme"),
        ("tokred-x86_64-pc-windows-msvc\\tokred.exe", NEW_BINARY),
    ])
}

// ---- names and urls -------------------------------------------------------------------------

#[test]
fn asset_names_follow_the_dist_convention() {
    assert_eq!(asset_name(LINUX), "tokred-x86_64-unknown-linux-musl.tar.gz");
    assert_eq!(
        asset_name("aarch64-apple-darwin"),
        "tokred-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(asset_name(WINDOWS), "tokred-x86_64-pc-windows-msvc.zip");
}

#[test]
fn release_urls_use_the_v_tag() {
    let expected = "https://github.com/sostenes198/tokred/releases/download/v0.4.0/tokred-x.tar.gz";
    assert_eq!(release_asset_url("0.4.0", "tokred-x.tar.gz"), expected);
    assert_eq!(release_asset_url("v0.4.0", "tokred-x.tar.gz"), expected);
}

#[test]
fn target_triples_cover_the_published_platforms() {
    assert_eq!(target_triple("x86_64", "linux").as_deref(), Some(LINUX));
    assert_eq!(
        target_triple("aarch64", "linux").as_deref(),
        Some("aarch64-unknown-linux-musl")
    );
    assert_eq!(
        target_triple("aarch64", "macos").as_deref(),
        Some("aarch64-apple-darwin")
    );
    assert_eq!(
        target_triple("x86_64", "macos").as_deref(),
        Some("x86_64-apple-darwin")
    );
    assert_eq!(target_triple("x86_64", "windows").as_deref(), Some(WINDOWS));
    assert_eq!(target_triple("riscv64", "linux"), None);
    assert_eq!(target_triple("x86_64", "freebsd"), None);
    assert!(
        current_target().is_some(),
        "the platform running the tests is supported"
    );
}

#[test]
fn checksum_files_are_parsed_leniently_but_strictly_on_the_digest() {
    let digest = "ab".repeat(32);
    assert_eq!(
        parse_checksum(&format!("{digest}  tokred.tar.gz\n")),
        Some(digest.clone())
    );
    assert_eq!(
        parse_checksum(&format!("{digest} *tokred.tar.gz")),
        Some(digest.clone())
    );
    assert_eq!(
        parse_checksum(&format!("{}\n", digest.to_uppercase())),
        Some(digest)
    );
    assert_eq!(parse_checksum(""), None);
    assert_eq!(parse_checksum("not-a-digest  file"), None);
    assert_eq!(parse_checksum(&"zz".repeat(32)), None);
    assert_eq!(parse_checksum(&"ab".repeat(31)), None);
}

// ---- self_update: success -------------------------------------------------------------------

#[test]
fn updates_a_unix_binary_in_place() {
    let archive = linux_archive();
    let asset = asset_name(LINUX);
    let downloader = downloader_for(LINUX, "0.4.0", &archive, &sidecar(&archive, &asset));
    let world = World::with_exe("/home/ana/.local/bin/tokred", b"old binary");

    let done = self_update(
        &downloader,
        &world,
        &world,
        Path::new("/home/ana/.local/bin/tokred"),
        "v0.4.0",
        LINUX,
    )
    .unwrap();

    assert_eq!(
        done,
        SelfUpdated {
            version: "0.4.0".into(),
            asset: asset.clone()
        }
    );
    assert_eq!(
        world.content("/home/ana/.local/bin/tokred").as_deref(),
        Some(NEW_BINARY)
    );
    assert_eq!(
        world.paths(),
        ["/home/ana/.local/bin/tokred"],
        "no staged file left"
    );
    assert_eq!(
        *world.executable.borrow(),
        [PathBuf::from("/home/ana/.local/bin/tokred.new")]
    );
    let url = release_asset_url("0.4.0", &asset);
    assert_eq!(
        *downloader.requested.borrow(),
        [url.clone(), format!("{url}.sha256")]
    );
}

#[test]
fn updates_a_windows_binary_by_parking_the_running_one() {
    let archive = windows_archive();
    let asset = asset_name(WINDOWS);
    let downloader = downloader_for(WINDOWS, "0.4.0", &archive, &sidecar(&archive, &asset));
    let exe = "C:/Users/Ana/AppData/Local/tokred/tokred.exe";
    let world = World::with_exe(exe, b"running binary");
    // A leftover from an earlier update must not block this one.
    world
        .files
        .borrow_mut()
        .insert(old_binary_path(Path::new(exe)), b"ancient".to_vec());

    let done = self_update(
        &downloader,
        &world,
        &world,
        Path::new(exe),
        "0.4.0",
        WINDOWS,
    )
    .unwrap();

    assert_eq!(done.asset, "tokred-x86_64-pc-windows-msvc.zip");
    assert_eq!(world.content(exe).as_deref(), Some(NEW_BINARY));
    assert_eq!(
        world.content(&format!("{exe}.old")).as_deref(),
        Some(&b"running binary"[..])
    );
    assert_eq!(world.paths().len(), 2);

    // The next run deletes the parked binary.
    assert!(cleanup_old_binary(&world, Path::new(exe)));
    assert!(!cleanup_old_binary(&world, Path::new(exe)), "already gone");
    assert_eq!(world.paths(), [exe]);
}

#[test]
fn cleanup_ignores_failures() {
    let world = World::with_exe("/x/tokred.exe.old", b"locked");
    world.fail_remove.set(true);
    assert!(!cleanup_old_binary(&world, Path::new("/x/tokred.exe")));
    assert_eq!(
        old_binary_path(Path::new("/x/tokred.exe")),
        PathBuf::from("/x/tokred.exe.old")
    );
}

// ---- self_update: verification --------------------------------------------------------------

#[test]
fn a_checksum_mismatch_aborts_before_touching_anything() {
    let archive = linux_archive();
    let wrong = format!("{}  {}\n", "0".repeat(64), asset_name(LINUX));
    let downloader = downloader_for(LINUX, "0.4.0", &archive, wrong.as_bytes());
    let world = World::with_exe("/bin/tokred", b"old binary");

    let err = self_update(
        &downloader,
        &world,
        &world,
        Path::new("/bin/tokred"),
        "0.4.0",
        LINUX,
    )
    .unwrap_err();

    let UpdateError::ChecksumMismatch { expected, actual } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(expected, &"0".repeat(64));
    assert_eq!(actual, &sha256_hex(&archive));
    assert!(err.to_string().contains("aborted"));
    assert_eq!(
        world.content("/bin/tokred").as_deref(),
        Some(&b"old binary"[..])
    );
    assert_eq!(world.paths(), ["/bin/tokred"]);
    assert!(world.executable.borrow().is_empty());
}

#[test]
fn a_tampered_archive_is_rejected_even_with_a_valid_looking_checksum_file() {
    let archive = linux_archive();
    let asset = asset_name(LINUX);
    let good_sidecar = sidecar(&archive, &asset);
    let mut tampered = archive.clone();
    tampered.push(0);
    let downloader = downloader_for(LINUX, "0.4.0", &tampered, &good_sidecar);
    let world = World::with_exe("/bin/tokred", b"old");
    let result = self_update(
        &downloader,
        &world,
        &world,
        Path::new("/bin/tokred"),
        "0.4.0",
        LINUX,
    );
    assert!(matches!(result, Err(UpdateError::ChecksumMismatch { .. })));
    assert_eq!(world.content("/bin/tokred").as_deref(), Some(&b"old"[..]));
}

#[test]
fn a_malformed_checksum_file_is_rejected() {
    let archive = linux_archive();
    let downloader = downloader_for(LINUX, "0.4.0", &archive, b"<html>rate limited</html>");
    let world = World::with_exe("/bin/tokred", b"old");
    let err = self_update(
        &downloader,
        &world,
        &world,
        Path::new("/bin/tokred"),
        "0.4.0",
        LINUX,
    )
    .unwrap_err();
    assert!(matches!(err, UpdateError::MalformedChecksum), "{err:?}");
    assert_eq!(world.content("/bin/tokred").as_deref(), Some(&b"old"[..]));
}

#[test]
fn download_failures_are_reported() {
    let world = World::with_exe("/bin/tokred", b"old");
    let exe = Path::new("/bin/tokred");

    let nothing = FakeDownloader::default();
    let err = self_update(&nothing, &world, &world, exe, "0.4.0", LINUX).unwrap_err();
    assert!(matches!(err, UpdateError::Download(_)), "{err:?}");

    let archive = linux_archive();
    let mut only_archive = FakeDownloader::default();
    only_archive
        .bodies
        .insert(release_asset_url("0.4.0", &asset_name(LINUX)), archive);
    let err = self_update(&only_archive, &world, &world, exe, "0.4.0", LINUX).unwrap_err();
    assert!(matches!(err, UpdateError::Download(_)), "{err:?}");
    assert_eq!(world.content("/bin/tokred").as_deref(), Some(&b"old"[..]));
}

// ---- self_update: archives ------------------------------------------------------------------

fn update_with_archive(target: &str, archive: &[u8]) -> Result<SelfUpdated, UpdateError> {
    let downloader = downloader_for(
        target,
        "0.4.0",
        archive,
        &sidecar(archive, &asset_name(target)),
    );
    let world = World::with_exe("/bin/tokred", b"old");
    let result = self_update(
        &downloader,
        &world,
        &world,
        Path::new("/bin/tokred"),
        "0.4.0",
        target,
    );
    if result.is_err() {
        assert_eq!(world.content("/bin/tokred").as_deref(), Some(&b"old"[..]));
        assert_eq!(world.paths(), ["/bin/tokred"]);
    }
    result
}

#[test]
fn an_archive_without_the_binary_is_rejected() {
    let tar = tar_gz(&[("pkg/README.md", b"x"), ("pkg/tokred-helper", b"y")]);
    let err = update_with_archive(LINUX, &tar).unwrap_err();
    assert!(
        matches!(&err, UpdateError::BinaryMissing(name) if name == "tokred"),
        "{err:?}"
    );
    assert!(err.to_string().contains("tokred"));

    let zip = zip_of(&[("pkg/tokred.exe.sig", b"y")]);
    let err = update_with_archive(WINDOWS, &zip).unwrap_err();
    assert!(
        matches!(&err, UpdateError::BinaryMissing(name) if name == "tokred.exe"),
        "{err:?}"
    );
}

#[test]
fn directories_named_like_the_binary_do_not_count() {
    let err = update_with_archive(LINUX, &tar_gz_with_directory_named_tokred()).unwrap_err();
    assert!(matches!(err, UpdateError::BinaryMissing(_)));
    // The zip fixture contains a directory entry `tokred.exe/`; only a file entry matches.
    let err = update_with_archive(WINDOWS, &zip_of(&[])).unwrap_err();
    assert!(matches!(err, UpdateError::BinaryMissing(_)));
}

#[test]
fn the_binary_is_found_at_the_top_level_too() {
    assert!(update_with_archive(LINUX, &tar_gz(&[("tokred", NEW_BINARY)])).is_ok());
    assert!(update_with_archive(WINDOWS, &zip_of(&[("tokred.exe", NEW_BINARY)])).is_ok());
}

#[test]
fn corrupt_archives_are_rejected_after_the_checksum_passes() {
    let err = update_with_archive(LINUX, b"this is not gzip").unwrap_err();
    assert!(matches!(err, UpdateError::Archive(_)), "{err:?}");
    assert!(err.to_string().contains("archive"));

    let err = update_with_archive(WINDOWS, b"this is not a zip").unwrap_err();
    assert!(matches!(err, UpdateError::Zip(_)), "{err:?}");
    assert!(err.to_string().contains("zip"));
}

// ---- self_update: replacing the binary ------------------------------------------------------

fn update_in(world: &World, target: &str, exe: &str) -> Result<SelfUpdated, UpdateError> {
    let archive = if is_windows(target) {
        windows_archive()
    } else {
        linux_archive()
    };
    let downloader = downloader_for(
        target,
        "0.4.0",
        &archive,
        &sidecar(&archive, &asset_name(target)),
    );
    self_update(&downloader, world, world, Path::new(exe), "0.4.0", target)
}

#[test]
fn a_failed_write_leaves_the_installation_untouched() {
    let world = World::with_exe("/bin/tokred", b"old");
    world.fail_write.set(true);
    let err = update_in(&world, LINUX, "/bin/tokred").unwrap_err();
    assert!(matches!(err, UpdateError::Fs(_)), "{err:?}");
    assert!(err.to_string().contains("injected"));
    assert_eq!(world.paths(), ["/bin/tokred"]);
}

#[test]
fn a_failed_chmod_removes_the_staged_file() {
    let world = World::with_exe("/bin/tokred", b"old");
    world.fail_chmod.set(true);
    assert!(update_in(&world, LINUX, "/bin/tokred").is_err());
    assert_eq!(world.paths(), ["/bin/tokred"]);
    assert_eq!(world.content("/bin/tokred").as_deref(), Some(&b"old"[..]));
}

#[test]
fn a_failed_unix_rename_removes_the_staged_file() {
    let world = World::with_exe("/bin/tokred", b"old");
    *world.fail_rename_to.borrow_mut() = Some(PathBuf::from("/bin/tokred"));
    assert!(update_in(&world, LINUX, "/bin/tokred").is_err());
    assert_eq!(world.paths(), ["/bin/tokred"]);
}

#[test]
fn windows_cannot_clear_a_locked_old_binary() {
    let world = World::with_exe("/bin/tokred.exe", b"old");
    world.fail_remove.set(true);
    assert!(update_in(&world, WINDOWS, "/bin/tokred.exe").is_err());
    // The staged file could not be removed either (same failure), but the exe is intact.
    assert_eq!(
        world.content("/bin/tokred.exe").as_deref(),
        Some(&b"old"[..])
    );
}

#[test]
fn windows_parking_failure_leaves_the_exe_in_place() {
    let world = World::with_exe("/bin/tokred.exe", b"old");
    *world.fail_rename_to.borrow_mut() = Some(PathBuf::from("/bin/tokred.exe.old"));
    assert!(update_in(&world, WINDOWS, "/bin/tokred.exe").is_err());
    assert_eq!(world.paths(), ["/bin/tokred.exe"]);
    assert_eq!(
        world.content("/bin/tokred.exe").as_deref(),
        Some(&b"old"[..])
    );
}

/// Swap whose injected rename failure happens once: the rollback rename (which targets the same
/// path as the failing one) must then succeed.
struct OneShot<'a>(&'a World);

impl BinarySwap for OneShot<'_> {
    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        let result = self.0.rename(from, to);
        if result.is_err() {
            *self.0.fail_rename_to.borrow_mut() = None;
        }
        result
    }

    fn set_executable(&self, path: &Path) -> Result<(), FsError> {
        self.0.set_executable(path)
    }
}

#[test]
fn windows_rolls_back_when_the_new_binary_cannot_be_moved_in() {
    let world = World::with_exe("/bin/tokred.exe", b"old");
    *world.fail_rename_to.borrow_mut() = Some(PathBuf::from("/bin/tokred.exe"));
    let archive = windows_archive();
    let downloader = downloader_for(
        WINDOWS,
        "0.4.0",
        &archive,
        &sidecar(&archive, &asset_name(WINDOWS)),
    );
    let swap = OneShot(&world);
    let result = self_update(
        &downloader,
        &world,
        &swap,
        Path::new("/bin/tokred.exe"),
        "0.4.0",
        WINDOWS,
    );
    assert!(result.is_err());
    assert_eq!(
        world.paths(),
        ["/bin/tokred.exe"],
        "old binary restored, nothing left over"
    );
    assert_eq!(
        world.content("/bin/tokred.exe").as_deref(),
        Some(&b"old"[..])
    );
}

// ---- the real file system adapter -----------------------------------------------------------

#[test]
fn the_std_adapter_renames_over_an_existing_file_and_marks_it_executable() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("tokred.new");
    let exe = dir.path().join("tokred");
    std::fs::write(&exe, b"old").unwrap();
    std::fs::write(&staged, b"new").unwrap();

    let swap = StdBinarySwap;
    swap.set_executable(&staged).unwrap();
    swap.rename(&staged, &exe).unwrap();

    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    assert!(!staged.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o111,
            0o111
        );
    }
}

#[test]
fn the_std_adapter_reports_failures_with_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let swap = StdBinarySwap;
    let err = swap.rename(&missing, &dir.path().join("to")).unwrap_err();
    assert!(err.to_string().contains("to"), "{err}");
    #[cfg(unix)]
    assert!(swap.set_executable(&missing).is_err());
}

#[test]
fn error_messages_are_readable() {
    let errors = [
        UpdateError::UnsupportedPlatform,
        UpdateError::MalformedChecksum,
        UpdateError::BinaryMissing("tokred".into()),
        UpdateError::from(SourceError::Invalid("boom".into())),
    ];
    for err in errors {
        assert!(!err.to_string().is_empty());
    }
}

#[test]
fn archive_paths_are_split_on_either_separator() {
    assert_eq!(base_name("a/b/tokred.exe"), "tokred.exe");
    assert_eq!(base_name("a\\b\\tokred.exe"), "tokred.exe");
    assert_eq!(base_name("tokred"), "tokred");
}
