**English** | [Português](SECURITY.pt-BR.md)

# Security policy

How to report a vulnerability in tokred, and how tokred is designed to limit the damage of one.

## Supported versions

Only the latest release receives security fixes. Update with `tokred update` (see [installation](docs/installation.md)).

## Report a vulnerability

Use GitHub's private reporting: open the **Security** tab of the repository and choose **Report a vulnerability**, or go to <https://github.com/sostenes198/tokred/security/advisories/new>. Do not open a public issue with exploit details.

Please include:

- the tokred version (`tokred --version`), your OS and how you installed it;
- what an attacker can do, and the steps to reproduce it;
- a proof of concept, if you have one, with any real secrets removed.

You can expect an acknowledgement of your report, a fix in a new release, and credit in the changelog and the advisory if you want it. Please give us time to release a fix before you disclose the problem publicly.

## Security model

- **Local only.** The proxy listens on `127.0.0.1` and refuses to start on any other address. It rejects (with `421`) any request whose `Host` header is not a loopback name (or the proxy's own address) on the proxy's own port, which blocks DNS-rebinding attacks from web pages.
- **Admin endpoints** (`/__tokred/status`, `/__tokred/shutdown`, `/__tokred/dashboard-token`) need a secret kept in a file in the data folder that only its owner can read (mode `0600` on macOS and Linux).
- **The dashboard** cannot be opened by another local user: `tokred dashboard` asks the proxy for a one-time token (valid for 60 seconds, usable once) and the browser trades it for an `HttpOnly`, `SameSite=Strict` cookie.
- **Credentials** (`Authorization`, `x-api-key`) are forwarded to the Anthropic API and never stored or logged.
- **Message contents** are stored only when you turn on `[proxy] log_bodies`, which is off by default.
- **Hooks fail open:** on any error, panic or timeout they exit 0 with no output, so tokred cannot block Claude Code.
- **Downloads are verified:** the installer and `tokred update` check the SHA-256 checksum of the archive before writing anything.
- **Uninstall:** `tokred uninstall` writes its final cleanup script to a new, uniquely named file in the temporary folder (owner-only permissions on macOS and Linux), not to a fixed name in a shared folder.

See [proxy](docs/proxy.md) and [dashboard](docs/dashboard.md) for the details.
