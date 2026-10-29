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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::io::Io;

const SHELL_SCRIPT: &str = include_str!("../../../../install/uninstall.sh");
const POWERSHELL_SCRIPT: &str = include_str!("../../../../install/uninstall.ps1");

/// The script of the operating system and the name it is saved under.
#[must_use]
pub fn script(windows: bool) -> (&'static str, &'static str) {
    if windows {
        (POWERSHELL_SCRIPT, "tokred-uninstall.ps1")
    } else {
        (SHELL_SCRIPT, "tokred-uninstall.sh")
    }
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
    let (text, name) = script(windows);
    let path: PathBuf = temp_dir.join(name);
    std::fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))?;
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
        assert_eq!(sh_name, "tokred-uninstall.sh");
        let (ps, ps_name) = script(true);
        assert!(ps.contains("-CleanupOnly") && ps.contains("WaitPid"));
        assert_eq!(ps_name, "tokred-uninstall.ps1");
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
        assert!(!dir.path().join("tokred-uninstall.sh").exists());

        // On Windows the arguments start with the PowerShell switches, and the file stays.
        finish(&mut io, true, 1, None, dir.path(), &|program, _| {
            assert_eq!(program, "powershell.exe");
            Ok(())
        })
        .unwrap();
        assert!(dir.path().join("tokred-uninstall.ps1").exists());

        // A failing launch is reported, and the unix script is still removed.
        let err = finish(&mut io, false, 1, None, dir.path(), &|_, _| {
            Err(anyhow::anyhow!("no sh"))
        })
        .unwrap_err();
        assert!(err.to_string().contains("no sh"));
        assert!(!dir.path().join("tokred-uninstall.sh").exists());

        // An unwritable folder says so.
        let missing = dir.path().join("no/such/dir");
        let err = finish(&mut io, false, 1, None, &missing, &|_, _| Ok(())).unwrap_err();
        assert!(err.to_string().contains("could not write"), "{err}");
    }
}
