# Security

## Privacy and boundaries

Focus Buddy runs as your desktop user. It opens the webcam after you start a
session, processes frames in memory, and discards them. Pause and Quit release
the camera and stop audio. Closing the window continues an active tray session.
The app does not record video, upload frames, send telemetry, or listen on a
network port. Camera frames are never saved by the screenshot smoke test.

Calibration values, a camera identity, preferences, local sound paths, and
website bookmarks are stored in your XDG user data directory. New settings
directories use mode 0700 and atomic settings files use 0600. Loading rejects
non-files, oversized data (128 KiB), and invalid values. Invalid existing files
are preserved. Existing parent directory permissions are not changed.

User inputs are passed as process arguments, without a shell. Camera selection
is checked against local V4L2 devices. Audio must be a local file; FFplay is
limited to the `file` protocol so local playlists cannot fetch remote media.
Website links accept only HTTP/HTTPS URLs without embedded credentials and
open in the default browser when clicked. Those sites are independent services.

Installation downloads Rust crates and two pinned models. Model downloads
permit HTTPS only, use unpredictable temporary files, and verify SHA-256 before
replacing a model. The app verifies the model hashes again before inference.
This provides integrity against accidental or unexpected changes; it does not
prove the safety of upstream model content. Sources and licenses are in
[models/README.md](models/README.md).

The Rust application forbids its own unsafe code. GTK, FFmpeg, ONNX Runtime,
the system runtime library override, and transitive dependencies remain trusted
native code. This application is not a sandbox: use audio files and runtime
libraries from sources you trust, and keep your system packages current.

## Dependency checks

Run `cargo audit` against the committed lockfile (install with
`cargo install cargo-audit --locked`). CI also checks the lockfile. On
2026-11-17, cargo-audit 0.23.0 reported **no known vulnerabilities** and three
**unmaintained** advisories: RUSTSEC-2026-0370 (`zvariant`),
RUSTSEC-2026-0371 (`zbus_names`), and RUSTSEC-2026-0372 (`zvariant_derive`).
They are transitive dependencies of `ksni` 0.3.7 through `zbus` 5. These crates
are being folded into zbus 6; the current tray library still uses the older
API. Recheck its next release. These warnings are documented, not suppressed;
the scan is not a formal security audit or a guarantee against new issues.

Keep routine bug reports free of saved settings, camera images, downloaded
sounds, local paths, or credentials. Use synthetic fixtures for reproductions.

## Reporting

Use GitHub's **Security → Report a vulnerability** if private reporting is
enabled on the repository hosting this code. If it is unavailable, open a
minimal issue asking the maintainer for a private contact channel, without
posting exploit details or personal data. No dedicated security response SLA
or independently audited release is provided.
