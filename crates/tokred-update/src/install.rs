//! How tokred was installed, judged from the path of its own executable (spec section 14).

/// The channel tokred was installed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// `brew install` (macOS and Linux).
    Homebrew,
    /// `winget install`.
    Winget,
    /// `scoop install`.
    Scoop,
    /// `cargo install`.
    Cargo,
    /// The shell/PowerShell installer script, or anything unrecognised: tokred updates itself.
    Script,
}

/// Detects the install method from the executable path.
///
/// Both `/` and `\` separators are accepted. Windows markers (`WinGet\Packages`, `scoop\apps`, ...)
/// are matched case-insensitively. Callers should pass the canonical path (symlinks resolved):
/// on macOS `current_exe()` may return `/opt/homebrew/bin/tokred`, a symlink into the Cellar.
#[must_use]
pub fn detect(path: &str) -> InstallMethod {
    let normalized = path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if normalized.contains("Cellar")
        || normalized.contains("linuxbrew")
        || normalized.starts_with("/opt/homebrew/")
    {
        InstallMethod::Homebrew
    } else if lower.contains("winget/packages") || lower.contains("winget/links") {
        InstallMethod::Winget
    } else if lower.contains("scoop/apps") || lower.contains("scoop/shims") {
        InstallMethod::Scoop
    } else if lower.contains(".cargo/bin") {
        InstallMethod::Cargo
    } else {
        InstallMethod::Script
    }
}

/// The command that upgrades tokred for `method`; `None` for [`InstallMethod::Script`], which
/// updates itself.
#[must_use]
pub const fn upgrade_command(method: InstallMethod) -> Option<&'static str> {
    match method {
        InstallMethod::Homebrew => Some("brew upgrade tokred"),
        InstallMethod::Winget => Some("winget upgrade tokred"),
        InstallMethod::Scoop => Some("scoop update tokred"),
        InstallMethod::Cargo => Some("cargo install tokred --force"),
        InstallMethod::Script => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_every_channel_with_both_separators() {
        let cases = [
            (
                "/opt/homebrew/Cellar/tokred/0.3.2/bin/tokred",
                InstallMethod::Homebrew,
            ),
            (
                "/usr/local/Cellar/tokred/0.3.2/bin/tokred",
                InstallMethod::Homebrew,
            ),
            (
                "/home/linuxbrew/.linuxbrew/bin/tokred",
                InstallMethod::Homebrew,
            ),
            ("/opt/homebrew/bin/tokred", InstallMethod::Homebrew),
            (
                r"C:\Users\Ana\AppData\Local\Microsoft\WinGet\Packages\x\tokred.exe",
                InstallMethod::Winget,
            ),
            (
                r"c:\users\ana\appdata\local\microsoft\winget\links\tokred.exe",
                InstallMethod::Winget,
            ),
            (
                "C:/Users/Ana/AppData/Local/Microsoft/WinGet/Links/tokred.exe",
                InstallMethod::Winget,
            ),
            (
                r"C:\Users\Ana\scoop\apps\tokred\current\tokred.exe",
                InstallMethod::Scoop,
            ),
            (r"C:\Users\Ana\SCOOP\Shims\tokred.exe", InstallMethod::Scoop),
            ("/home/ana/.cargo/bin/tokred", InstallMethod::Cargo),
            (r"C:\Users\Ana\.cargo\bin\tokred.exe", InstallMethod::Cargo),
            ("/home/ana/.local/bin/tokred", InstallMethod::Script),
            (
                r"C:\Users\Ana\AppData\Local\tokred\tokred.exe",
                InstallMethod::Script,
            ),
            ("/usr/local/bin/tokred", InstallMethod::Script),
            ("", InstallMethod::Script),
        ];
        for (path, expected) in cases {
            assert_eq!(detect(path), expected, "{path}");
        }
    }

    #[test]
    fn upgrade_commands_match_the_spec() {
        assert_eq!(
            upgrade_command(InstallMethod::Homebrew),
            Some("brew upgrade tokred")
        );
        assert_eq!(
            upgrade_command(InstallMethod::Winget),
            Some("winget upgrade tokred")
        );
        assert_eq!(
            upgrade_command(InstallMethod::Scoop),
            Some("scoop update tokred")
        );
        assert_eq!(
            upgrade_command(InstallMethod::Cargo),
            Some("cargo install tokred --force")
        );
        assert_eq!(upgrade_command(InstallMethod::Script), None);
    }
}
