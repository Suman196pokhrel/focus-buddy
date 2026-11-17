# Contributing

Use the Linux prerequisites in the [user guide](docs/guide.md#other-linux-distributions). Keep changes small and
preserve saved calibration, optional notebook mode, continuous reminder audio,
bounded frame handling, and the window/tray lifecycle.

```sh
./bin/fetch-models
cargo build --locked
cargo fmt --check
./bin/test
./target/debug/focus-buddy --check
```

`bin/test` needs Python 3 and ffplay in addition to the build dependencies.
It checks shell syntax, installer failure paths and model download integrity,
Rust unit tests (including real audio with SDL dummy output), and Clippy.
No unit test opens a webcam or installs system packages. `--check` verifies
model hashes and runs blank-frame inference; it needs ONNX Runtime but no display.

The optional native integration test requires a running Hyprland desktop,
`hyprctl`, D-Bus, and system Python with PyGObject:

```sh
dbus-run-session -- /usr/bin/python tests/desktop.py
dbus-run-session -- /usr/bin/python tests/desktop.py --screenshot
```

It uses a test tray host, separate settings, and disabled portal/accessibility
discovery on its isolated bus. It checks launcher activation, hide/reopen,
notebook persistence, tray recovery, and quitting without opening the camera.
The screenshot command writes `target/ui-smoke.png`; use this fixture instead
of exposing your personal settings. Other useful manual checks:

```sh
./target/debug/focus-buddy --bench path/to/face.jpg
./target/debug/focus-buddy --camera-check /dev/video0
```

The first benchmarks a still image, without measuring live gaze accuracy.
The second explicitly opens the camera for five seconds, reports counts, and
releases it without recording. Validate live calibration and sound recovery
manually after changes to detection or camera capture.

`ui.rs` owns GTK controls, `tray.rs` owns the StatusNotifierItem menu,
`camera.rs` owns capture, `vision.rs` runs models, `focus.rs` handles positions
and reminder timing, `settings.rs` saves preferences, and `audio.rs` owns the
player process. Use existing helpers and avoid adding dependencies for small tasks.

Build an Arch package with `cargo fetch --locked`, followed by `makepkg --force`
in `packaging/`. Never build as root. The package includes model licenses.
Run `cargo audit` when updating dependencies; see [SECURITY.md](SECURITY.md).

Do not commit downloaded models, build output, personal settings, camera
frames, sounds downloaded from third-party sites, or credentials. Keep
`Cargo.lock` and model license notices tracked. Report security issues using
the private reporting guidance in SECURITY.md.
