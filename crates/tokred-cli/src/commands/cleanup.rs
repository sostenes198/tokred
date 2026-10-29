//! The last step of `tokred uninstall`: the tokred binary, its folder, the PATH entry and the
//! `claude` wrapper in the shell profiles.
//!
//! The work is done by the uninstall script that ships inside this binary (the same file users
//! can run by hand), in its `--cleanup-only` mode, so the logic exists once and the installer
//! tests of CI cover it on every operating system.
//!
//! On Linux and macOS the script runs to the end before `tokred` exits (a running program can
//! delete its own file). On Windows a running exe cannot delete itself, so the script is started
//! detached and waits for this process to exit first.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::io::Io;

const SHELL_SCRIPT: &str = include_str!("../../../../install/uninstall.sh");
const POWERSHELL_SCRIPT: &str = include_str!("../../../../install/uninstall.ps1");

/// The script of the operating system and the extension of the file it is saved as.
#[must_use]
pub fn script(windows: bool) -> (&'static str, &'static str) {
    if windows {
        (POWERSHELL_SCRIPT, "ps1")
    } else {
        (SHELL_SCRIPT, "sh")
    }
}

/// Creates `path` for writing, which must not exist yet (so nothing planted there is followed or
/// reused), readable and runnable by its owner only.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    options.open(path)
}

/// The program and arguments that run `script` in its cleanup-only mode.
///
/// `install_home` is the folder the install script put the binary in (`None` when this binary
/// was not installed by the script: then the binary is left alone). On Windows the script also
/// waits for process `pid` to exit; on the other systems it runs before this process exits and
/// must not wait for it.
#[must_use]
pub fn command_line(
    windows: bool,
    script: &Path,
    pid: u32,
    install_home: Option<&Path>,
) -> (&'static str, Vec<String>) {
    let text = |path: &Path| path.display().to_string();
    if windows {
        let mut args = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
            .map(str::to_owned)
            .to_vec();
        args.extend([text(script), "-CleanupOnly".to_owned()]);
        args.extend(["-WaitPid".to_owned(), pid.to_string()]);
        args.extend(
            install_home
                .into_iter()
                .flat_map(|home| ["-TokredHome".to_owned(), text(home)]),
        );
        ("powershell.exe", args)
    } else {
        let mut args = vec![text(script), "--cleanup-only".to_owned()];
        args.extend(
            install_home
                .into_iter()
                .flat_map(|home| ["--home".to_owned(), text(home)]),
        );
        ("sh", args)
    }
}

/// Saves the script in `temp_dir` and runs it through `launch(program, args)`.
pub fn finish(
    io: &mut Io,
    windows: bool,
    pid: u32,
    install_home: Option<&Path>,
    temp_dir: &Path,
    launch: &dyn Fn(&str, &[String]) -> Result<()>,
) -> Result<()> {
    let note = [
        "Removing the tokred binary, its PATH entry and the `claude` wrapper...",
        "The tokred binary, its PATH entry and the `claude` wrapper are removed as soon as this command ends.",
    ][usize::from(windows)];
    writeln!(io.out(), "{note}")?;
    let (text, extension) = script(windows);
    // The temporary folder is shared on Linux and macOS: a fixed name could be planted by another
    // user, or collide with a second uninstall running at the same time.
    let name = format!(
        "tokred-uninstall-{pid}-{}.{extension}",
        &crate::daemon::random_hex()[..16]
    );
    let path: PathBuf = temp_dir.join(name);
    create_private(&path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .with_context(|| format!("could not write {}", path.display()))?;
    let (program, args) = command_line(windows, &path, pid, install_home);
    let result = launch(program, &args);
    if !windows {
        // Done by now. On Windows the script is still waiting for this process to exit.
        let _ = std::fs::remove_file(&path);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::io::testing::scripted;

    #[test]
    fn each_system_gets_its_own_script() {
        let (sh, sh_name) = script(false);
        assert!(sh.starts_with("#!/bin/sh") && sh.contains("--cleanup-only"));
        assert_eq!(sh_name, "sh");
        let (ps, ps_name) = script(true);
        assert!(ps.contains("-CleanupOnly") && ps.contains("WaitPid"));
        assert_eq!(ps_name, "ps1");
    }

    #[test]
    fn the_command_lines_name_the_script_the_pid_and_the_install_folder() {
        let script_path = Path::new("/tmp/x/tokred-uninstall.sh");
        let home = Path::new("/home/u/.tokred");
        let (program, args) = command_line(false, script_path, 42, Some(home));
        assert_eq!(program, "sh");
        assert_eq!(
            args,
            [
                "/tmp/x/tokred-uninstall.sh",
                "--cleanup-only",
                "--home",
                "/home/u/.tokred"
            ]
        );
        // No install folder (a binary the script did not install): it is not named.
        let (_, args) = command_line(false, script_path, 42, None);
        assert_eq!(args, ["/tmp/x/tokred-uninstall.sh", "--cleanup-only"]);

        let (program, args) = command_line(true, script_path, 42, Some(home));
        assert_eq!(program, "powershell.exe");
        assert_eq!(
            args,
            [
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "/tmp/x/tokred-uninstall.sh",
                "-CleanupOnly",
                "-WaitPid",
                "42",
                "-TokredHome",
                "/home/u/.tokred"
            ]
        );
        let (_, args) = command_line(true, script_path, 7, None);
        assert_eq!(&args[5..], ["-CleanupOnly", "-WaitPid", "7"]);
    }

    /// The names of the files in `dir`.
    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_script_is_saved_run_and_removed_on_unix_but_kept_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let (mut io, _) = scripted(false, "");
        let seen = RefCell::new(Vec::new());
        let launch = |program: &str, args: &[String]| -> Result<()> {
            // At launch time the script exists with the shell script's text.
            let saved = std::fs::read_to_string(&args[0]).unwrap_or_default();
            seen.borrow_mut().push((program.to_owned(), saved.len()));
            Ok(())
        };
        finish(&mut io, false, 1, None, dir.path(), &launch).unwrap();
        assert_eq!(seen.borrow()[0], ("sh".to_owned(), SHELL_SCRIPT.len()));
        assert!(files_in(dir.path()).is_empty());

        // On Windows the arguments start with the PowerShell switches, and the file stays.
        finish(&mut io, true, 1, None, dir.path(), &|program, _| {
            assert_eq!(program, "powershell.exe");
            Ok(())
        })
        .unwrap();
        let kept = files_in(dir.path());
        assert_eq!(kept.len(), 1, "{kept:?}");
        let named = kept[0].starts_with("tokred-uninstall-1-");
        assert!(
            named && Path::new(&kept[0]).extension().is_some_and(|e| e == "ps1"),
            "{kept:?}"
        );

        // A failing launch is reported, and the unix script is still removed.
        let err = finish(&mut io, false, 1, None, dir.path(), &|_, _| {
            Err(anyhow::anyhow!("no sh"))
        })
        .unwrap_err();
        assert!(err.to_string().contains("no sh"));
        assert_eq!(files_in(dir.path()), kept);

        // An unwritable folder says so.
        let missing = dir.path().join("no/such/dir");
        let err = finish(&mut io, false, 1, None, &missing, &|_, _| Ok(())).unwrap_err();
        assert!(err.to_string().contains("could not write"), "{err}");
    }

    #[test]
    fn two_uninstalls_at_once_never_share_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut io, _) = scripted(false, "");
        // The script of a Windows run stays, so the names of several runs can be compared.
        for _ in 0..3 {
            finish(&mut io, true, 7, None, dir.path(), &|_, _| Ok(())).unwrap();
        }
        let names = files_in(dir.path());
        assert_eq!(names.len(), 3, "every run has a name of its own: {names:?}");
    }

    #[test]
    fn a_file_that_is_already_there_is_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("planted");
        std::fs::write(&path, "planted").unwrap();
        assert!(create_private(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "planted");
    }

    #[cfg(unix)]
    #[test]
    fn the_script_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script");
        create_private(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
