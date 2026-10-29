//! Starting the background proxy through PowerShell on Windows.
//!
//! A process started with `CreateProcess` inherits every inheritable handle of its parent, so a
//! detached proxy would keep the pipes of a caller that captures the output of `tokred proxy
//! start` (`out=$(tokred proxy start)`, a CI step) open, and the caller would wait for it to
//! exit. `Start-Process` starts the program with shell semantics and without those handles.
//!
//! The command line is built by pure functions that are tested on every operating system; only
//! the choice of using them is Windows-specific (see `spawn_detached`).
#![cfg_attr(not(windows), allow(dead_code))]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// The PowerShell that ships with every supported Windows.
pub const POWERSHELL: &str = "powershell.exe";

/// Quotes `arg` the way Windows programs (`CommandLineToArgvW`) read it back: untouched when it
/// has nothing special, else in double quotes with the backslashes before a quote doubled.
fn windows_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// A PowerShell single-quoted string: nothing inside is expanded, a quote is doubled.
fn powershell_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The arguments of `powershell.exe` that start `exe args...` hidden and print its process id.
///
/// `-ArgumentList` joins its items with spaces and does no quoting of its own, so an item with
/// a space is quoted first ([`windows_argument`]).
pub fn powershell_args(exe: &Path, args: &[&str]) -> Vec<String> {
    let mut script = format!(
        "Start-Process -FilePath {}",
        powershell_literal(&exe.display().to_string())
    );
    if !args.is_empty() {
        let items: Vec<String> = args
            .iter()
            .map(|arg| powershell_literal(&windows_argument(arg)))
            .collect();
        script.push_str(" -ArgumentList ");
        script.push_str(&items.join(","));
    }
    script.push_str(" -WindowStyle Hidden -PassThru | Select-Object -ExpandProperty Id");
    ["-NoProfile", "-NonInteractive", "-Command"]
        .into_iter()
        .map(str::to_owned)
        .chain([script])
        .collect()
}

/// The process id on the first line a launcher printed.
fn parse_pid(line: &str) -> Result<u32> {
    line.trim().parse().with_context(|| {
        format!(
            "the launcher did not print a process id (it printed `{}`)",
            line.trim()
        )
    })
}

/// The process id of `exe args...` started through PowerShell, or `None` when PowerShell cannot
/// be used (the caller then starts the process itself).
#[cfg(windows)]
pub fn start_hidden(exe: &Path, args: &[&str]) -> Option<u32> {
    pid_from_launcher(POWERSHELL, &powershell_args(exe, args)).ok()
}

/// Only Windows needs the detour.
#[cfg(not(windows))]
pub fn start_hidden(_exe: &Path, _args: &[&str]) -> Option<u32> {
    None
}

#[cfg(windows)]
fn without_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn without_console(_command: &mut Command) {}

/// Runs `program args...`, which must print the id of the process it started on its first line,
/// and returns that id. Only the first line is read, so a launcher that leaves its output pipe
/// open in the process it started does not hold this call up.
pub fn pid_from_launcher(program: &str, args: &[String]) -> Result<u32> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    without_console(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("could not run {program}"))?;
    let mut line = String::new();
    let stdout = child.stdout.take().context("the launcher has no output")?;
    let read = BufReader::new(stdout).read_line(&mut line);
    let _ = child.wait();
    read.with_context(|| format!("could not read the output of {program}"))?;
    parse_pid(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_for_the_windows_command_line() {
        assert_eq!(windows_argument("--foreground"), "--foreground");
        assert_eq!(windows_argument("127.0.0.1:1029"), "127.0.0.1:1029");
        assert_eq!(windows_argument(""), "\"\"");
        assert_eq!(windows_argument("my data"), "\"my data\"");
        assert_eq!(windows_argument("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(windows_argument("C:\\my dir\\"), "\"C:\\my dir\\\\\"");
        assert_eq!(windows_argument("a\\\"b c"), "\"a\\\\\\\"b c\"");
        assert_eq!(windows_argument("tab\there"), "\"tab\there\"");
    }

    #[test]
    fn powershell_literals_double_their_quotes() {
        assert_eq!(powershell_literal("plain"), "'plain'");
        assert_eq!(powershell_literal("it's"), "'it''s'");
    }

    #[test]
    fn the_powershell_command_starts_the_program_hidden_and_prints_its_id() {
        let args = powershell_args(
            Path::new(r"C:\Users\Ann Lee\tokred.exe"),
            &["proxy", "start", "--foreground", "--bind", "127.0.0.1:1029"],
        );
        assert_eq!(&args[..3], ["-NoProfile", "-NonInteractive", "-Command"]);
        assert_eq!(args.len(), 4);
        assert_eq!(
            args[3],
            "Start-Process -FilePath 'C:\\Users\\Ann Lee\\tokred.exe' -ArgumentList 'proxy','start','--foreground','--bind','127.0.0.1:1029' -WindowStyle Hidden -PassThru | Select-Object -ExpandProperty Id"
        );
    }

    #[test]
    fn paths_and_arguments_with_spaces_and_quotes_survive() {
        let args = powershell_args(Path::new("C:\\it's here\\t.exe"), &["--data", "my dir"]);
        let script = &args[3];
        assert!(
            script.contains("-FilePath 'C:\\it''s here\\t.exe'"),
            "{script}"
        );
        assert!(
            script.contains("-ArgumentList '--data','\"my dir\"'"),
            "{script}"
        );
    }

    #[test]
    fn no_arguments_means_no_argument_list() {
        let args = powershell_args(Path::new("t.exe"), &[]);
        assert!(!args[3].contains("-ArgumentList"));
        assert!(args[3].starts_with("Start-Process -FilePath 't.exe' -WindowStyle Hidden"));
    }

    #[test]
    fn a_process_id_is_read_from_the_first_line() {
        assert_eq!(parse_pid("4242\r\n").unwrap(), 4242);
        assert!(parse_pid("").is_err());
        let error = parse_pid("Start-Process : The system cannot find the file\n").unwrap_err();
        assert!(error.to_string().contains("did not print a process id"));
    }

    #[cfg(unix)]
    fn sh(script: &str) -> Result<u32> {
        pid_from_launcher("sh", &["-c".to_owned(), script.to_owned()])
    }

    #[cfg(unix)]
    #[test]
    fn the_launcher_prints_the_pid_and_may_leave_a_background_process_with_its_pipe() {
        assert_eq!(sh("echo 4242").unwrap(), 4242);
        // The grandchild keeps the output pipe open for two seconds: only the first line counts.
        let started = std::time::Instant::now();
        assert_eq!(sh("echo 7; (sleep 2 &) ; true").unwrap(), 7);
        assert!(started.elapsed() < std::time::Duration::from_millis(1500));
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_that_fails_or_prints_garbage_is_an_error() {
        assert!(sh("echo oops").is_err());
        let unreadable = sh("printf '\\377\\n'").unwrap_err();
        assert!(unreadable.to_string().contains("could not read the output"));
        assert!(sh("exit 3").is_err());
        let missing = pid_from_launcher("/no/such/launcher", &[]).unwrap_err();
        assert!(missing.to_string().contains("could not run"));
    }
}
