#!/bin/sh
set -eu

case "${1:-}" in
    --help|-h)
        printf '%s\n' 'Usage: ./install.sh [--check]' \
            'Build and install Focus Buddy on Arch Linux / Omarchy.' \
            '--check checks system dependencies without changing anything.'
        exit 0 ;;
    ''|--check) ;;
    *) printf '%s\n' 'Unknown option. Use ./install.sh --help.' >&2; exit 2 ;;
esac
test "$#" -le 1 || { printf '%s\n' 'Use ./install.sh --help.' >&2; exit 2; }
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
if test "$(uname -s)" != Linux || ! command -v pacman >/dev/null 2>&1; then
    printf '%s\n' 'This installer supports Arch Linux / Omarchy. See README.md for other Linux systems.' >&2
    exit 1
fi
if test "$(id -u)" -eq 0; then
    printf '%s\n' 'Run ./install.sh as your regular user. Only package installation needs administrator access.' >&2
    exit 1
fi

if missing=$(pacman -T base-devel rust cargo pkgconf curl 'gtk4>=4.10' ffmpeg 'onnxruntime>=1.22'); then
    missing=''
else
    status=$?
    test "$status" -eq 127 || exit "$status"
fi
if test "${1:-}" = --check; then
    if test -n "$missing"; then
        printf 'Missing system dependencies:\n%s\nRun ./install.sh to install them.\n' "$missing"
        exit 1
    fi
    printf '%s\n' 'System dependencies are ready. Run ./install.sh to build and install.'
    exit 0
fi

elevate() {
    if test -t 0 && command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    elif command -v pkexec >/dev/null 2>&1; then
        pkexec "$@"
    else
        printf '%s\n' 'Administrator access needs sudo in a terminal or a working Polkit authentication agent.' >&2
        exit 1
    fi
}
if test -n "$missing"; then
    # Choose the CPU provider only when no compatible ONNX Runtime is installed.
    set --
    for dependency in $missing; do
        case "$dependency" in
            cargo|rust) package=rust ;;
            'onnxruntime>=1.22') package=onnxruntime-cpu ;;
            'gtk4>=4.10') package=gtk4 ;;
            base-devel|pkgconf|curl|ffmpeg) package=$dependency ;;
            *) printf 'Unexpected dependency: %s\n' "$dependency" >&2; exit 1 ;;
        esac
        case " $* " in *" $package "*) ;; *) set -- "$@" "$package" ;; esac
    done
    elevate pacman -S --needed -- "$@"
fi

printf '%s\n' 'Downloading verified models and Rust dependencies, then building a native package. This can take several minutes.'
sh ./bin/fetch-models
cargo fetch --locked
cd packaging
makepkg --force
package=$(makepkg --packagelist)
test -f "$package" || { printf '%s\n' 'The package was not produced.' >&2; exit 1; }
elevate pacman -U -- "$package"
/usr/bin/focus-buddy --check
printf '%s\n' 'Installed. Open Focus Buddy from your app launcher. Start setup to turn on the camera.'
