# Focus Buddy

A small native Rust app that plays a sound when your webcam estimates you have
looked away from your working positions. Look back and the sound stops.

![Focus Buddy desktop interface](assets/screenshot.png)

Camera frames are processed on your computer and discarded. No recordings,
uploads, accounts, or telemetry. You choose when to turn the camera on.

## Install on Arch Linux / Omarchy

**Tested on Arch Linux / Omarchy, x86-64.** You need a webcam, two screens,
an audio output, an internet connection for installation, and administrator
access to install system packages. The first build takes several minutes;
later builds reuse downloaded dependencies. There is no prebuilt binary release yet.

1. On GitHub, choose **Code → Download ZIP**, extract it, and open a terminal
   in the extracted `focus-buddy` folder. Or clone the repository using its
   **Code** URL and open that folder.

   ```sh
   git clone https://github.com/Suman196pokhrel/focus-buddy.git
   cd focus-buddy
   ```

2. On Arch, bring your system up to date first with `sudo pacman -Syu`.
3. Run:

   ```sh
   ./install.sh
   ```

   If an extracted archive loses executable permissions, use `sh install.sh`.
   Run as your regular user; the installer asks for administrator access only
   when installing packages. It installs missing dependencies, downloads and
   verifies the two models, builds an Arch package, installs it with pacman,
   and checks inference without opening your webcam.
4. Find **Focus Buddy** in your app launcher. You can also run `focus-buddy`.

For a dependency check that changes nothing: `./install.sh --check`.
The installer uses the CPU ONNX Runtime package only if a compatible provider
is not already installed. It does not add startup services or change desktop settings.

## Your first session

1. Choose your camera and click **Start setup**. **Show camera preview** is
   optional; the preview is off by default.
2. Look naturally at your first screen and click its **Set position** button.
   Keep looking there for about six seconds. Repeat for **Screen 2**.
   Reminders begin once both screen positions are saved.
3. If you take handwritten notes, turn on **Notebook mode** in Settings and
   set the notebook position. Turn it off for sessions without notes; its
   saved position is kept. You can also set a **Phone** position where you
   normally hold your phone. Keep phone and notebook positions apart.
4. Choose a local sound with **Choose sound…**, then **Preview sound**.
   WAV, MP3, and OGG are supported through FFmpeg. **Use chime** restores the
   included sound. Website links open in your browser: download a sound there,
   then choose the downloaded file in Focus Buddy. External sites have their
   own privacy policies and audio licenses.
5. Choose **Remind me after** in Settings (six seconds by default). Your sound
   loops after that delay until a saved working position is recognized.
   A new distraction starts a fresh delay. **Fine-tune detection** lets you
   allow more head movement if normal movement triggers reminders.

**Pause session** turns the camera off and stops the sound. **Start session**
uses your saved positions next time. Repositioning your camera, screens,
notebook, or chair may require **Update position**. Switching cameras clears
the saved positions; resetting them asks for confirmation.

### Window and tray

Closing the window or choosing **Hide to tray** keeps the current session
running. Click the tray icon or open Focus Buddy from the launcher to return.
Hover over the icon for status; right-click to open, start/pause, toggle
notebook mode, or quit. Omarchy may put the icon in its expandable tray drawer.
If the tray host disappears, the window reopens.

Choose **Quit** or press **Ctrl+Q** to exit completely. **Space** pauses a
running session when keyboard focus is outside a control. There is no autostart.

## What detection can tell

Focus Buddy estimates look direction from facial landmarks and approximate eye
position. It does not identify phone objects, recognize handwriting, confirm
scrolling, or measure mental focus. Browsing distractions on a working screen
will still look like work to this app.

Poor light, glasses, reflections, closed eyes, multiple faces, or overlapping
working positions can cause uncertain readings. Uncertain readings do not
start a reminder; once a reminder is sounding, it keeps looping until a working
position is recognized. Turning fully away before the first alert may go
unalerted. Stale frames, camera errors, pausing, and position setup stop the
sound. Live desk accuracy has not been measured across users.

Analysis is capped at 10 frames per second, with one inference thread per
model and a single latest-frame slot. Hidden previews are not rendered.
Rust helps with memory safety; GTK, FFmpeg, and ONNX Runtime still determine
much of the app's memory use. No resource benchmark is implied.

## Update or uninstall

For a Git checkout, run these commands in the project folder:

```sh
git pull --ff-only
./install.sh
```

For a ZIP download, download/extract the newer version and run its installer.
Quit and reopen an already-running copy after upgrading. Your saved positions
and preferences stay in your user data directory.

To uninstall the app and models:

```sh
sudo pacman -R focus-buddy
```

Personal settings remain at `~/.local/share/focus-buddy/state.json`, or
`$XDG_DATA_HOME/focus-buddy/state.json` if that variable is set. Remove that
folder manually only if you also want to delete your saved setup.

## Other Linux distributions / source builds

The automatic installer targets Arch and Omarchy. Other Linux desktops need
**GTK 4.10+** and its development files, **Rust/Cargo 1.89+**, a C build toolchain,
`pkg-config`, `curl`, `sha256sum`, **FFmpeg with ffplay**, and the CPU shared
library from [ONNX Runtime 1.22+](https://onnxruntime.ai/docs/install/).
Use your distribution's packages where available. Windows and macOS are not supported.

Once prerequisites are installed, from the project folder:

```sh
./bin/fetch-models
./bin/build
./target/release/focus-buddy --check
./target/release/focus-buddy
```

The default runtime library path is `/usr/lib/libonnxruntime.so`. If your
distribution uses another path, set `ORT_DYLIB_PATH` to its absolute `.so` path
before running the app. `FOCUS_BUDDY_MODELS` can override the model directory.
Tray support requires a StatusNotifierItem host. The full desktop flow has
only been tested on Omarchy/Hyprland; other desktops may need additional tray support.

## Troubleshooting

- **No camera or no picture:** close other camera apps, reconnect your webcam,
  and click Refresh. A camera may expose multiple entries; try another entry.
  Check your distribution's camera permissions rather than making devices
  world-writable. Capture currently requires a 640×480 V4L2 stream.
- **Sound keeps playing:** look back at a saved working position, or click
  Pause session. Update positions after moving your desk setup.
- **Sound will not play:** preview the included chime and check your audio
  output. Keep custom sounds on disk; removing or moving the file breaks its
  saved selection. Network playlists are intentionally blocked.
- **Model/runtime error:** run `focus-buddy --check` in a terminal. On Arch,
  rerun the installer to restore models and missing dependencies. For a source
  build, run `./bin/fetch-models` and check `ORT_DYLIB_PATH`.
- **Invalid saved settings:** the original file is preserved and saving is
  blocked. Quit, rename `state.json` to `state.json.backup`, then reopen and
  set up your positions again. Keep the backup private.

## Development and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for build checks and the optional desktop
integration test, [SECURITY.md](SECURITY.md) for security boundaries and reporting,
and [models/README.md](models/README.md) for model sources and licenses.
The application is MIT licensed; third-party dependencies and models retain
their own licenses. `Cargo.lock` is committed for reproducible dependency versions.
