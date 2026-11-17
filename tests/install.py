"""Exercise installer failures and model integrity without changing the system."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class InstallTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "target").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="installer test ", dir=ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.tools = self.repo / "tools"
        self.tools.mkdir()
        (self.repo / "bin").mkdir()
        (self.repo / "models").mkdir()
        (self.repo / "packaging").mkdir()
        shutil.copy2(ROOT / "install.sh", self.repo)
        self.env = {**os.environ, "PATH": f"{self.tools}:{os.environ['PATH']}",
                    "MOCK_LOG": str(self.repo / "commands")}
        self.tool("uname", 'printf "%s\\n" "${MOCK_OS:-Linux}"')
        self.tool("id", 'printf "%s\\n" "${MOCK_UID:-1000}"')
        self.tool("pacman", 'if test "$1" = -T; then\n'
                  '  if test -n "${MOCK_MISSING:-}"; then printf "%s\\n" "$MOCK_MISSING"; exit 127; fi\n'
                  'else printf "%s\\n" "$@" >> "$MOCK_LOG"; fi')
        self.tool("pkexec", 'printf "%s\\n" "$@" >> "$MOCK_LOG"; exit "${MOCK_AUTH_STATUS:-0}"')
        self.tool("sudo", 'printf "%s\\n" "$@" >> "$MOCK_LOG"; exit "${MOCK_AUTH_STATUS:-0}"')
        self.tool("cargo", 'printf "%s\\n" "$@" >> "$MOCK_LOG"')
        self.tool("makepkg", 'printf "%s\\n" "build failed" >&2; exit 7')
        (self.repo / "bin/fetch-models").write_text('#!/bin/sh\nexit 0\n')
        (self.repo / "bin/fetch-models").chmod(0o755)

    def tool(self, name, script):
        path = self.tools / name
        path.write_text(f"#!/bin/sh\nset -eu\n{script}\n")
        path.chmod(0o755)

    def run_install(self, *args, **env):
        return subprocess.run([str(self.repo / "install.sh"), *args],
                              env={**self.env, **env}, capture_output=True, text=True)

    def test_preflight_never_installs_or_downloads(self):
        self.assertEqual(self.run_install("--check").returncode, 0)
        result = self.run_install("--check", MOCK_MISSING="onnxruntime>=1.22")
        self.assertEqual(result.returncode, 1)
        self.assertIn("onnxruntime>=1.22", result.stdout)
        self.assertFalse((self.repo / "commands").exists())

    def test_rejects_root_unsupported_os_and_unknown_options(self):
        self.assertIn("regular user", self.run_install(MOCK_UID="0").stderr)
        self.assertIn("other Linux", self.run_install(MOCK_OS="Darwin").stderr)
        self.assertEqual(self.run_install("--surprise").returncode, 2)
        self.assertEqual(self.run_install("--check", "extra").returncode, 2)
        self.assertEqual(self.run_install("--help", MOCK_UID="0").returncode, 0)
        self.assertFalse((self.repo / "commands").exists())

    def test_missing_runtime_selects_cpu_and_build_failure_stops_install(self):
        result = self.run_install(MOCK_MISSING="rust\ncargo\nonnxruntime>=1.22")
        self.assertEqual(result.returncode, 7)
        commands = (self.repo / "commands").read_text().splitlines()
        self.assertEqual(commands[:7], ["pacman", "-S", "--needed", "--", "rust", "onnxruntime-cpu", "fetch"])
        self.assertNotIn("-U", commands)

    def test_denied_authentication_stops_before_download_and_build(self):
        result = self.run_install(MOCK_MISSING="ffmpeg", MOCK_AUTH_STATUS="126")
        self.assertEqual(result.returncode, 126)
        self.assertNotIn("fetch", (self.repo / "commands").read_text())

    def test_built_package_path_with_spaces_is_one_argument(self):
        self.tool("makepkg", 'if test "$1" = --packagelist; then printf "%s/test package.pkg.tar.zst\\n" "$PWD"; '
                  'else touch "test package.pkg.tar.zst"; fi')
        result = self.run_install(MOCK_AUTH_STATUS="126")
        self.assertEqual(result.returncode, 126)
        commands = (self.repo / "commands").read_text().splitlines()
        self.assertEqual(commands[-4:], ["pacman", "-U", "--", str(self.repo / "packaging/test package.pkg.tar.zst")])

    def test_corrupt_model_download_preserves_previous_file_and_cleans_up(self):
        shutil.copy2(ROOT / "bin/fetch-models", self.repo / "bin/fetch-models")
        model = self.repo / "models/yunet.onnx"
        model.write_bytes(b"previous file")
        self.tool("curl", 'printf "%s\\n" "$@" >> "$MOCK_LOG"\n'
                  'for argument do destination=$argument; done\nprintf "corrupt download" > "$destination"')
        result = subprocess.run([str(self.repo / "bin/fetch-models")], env=self.env, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(model.read_bytes(), b"previous file")
        self.assertEqual(list((self.repo / "models").glob(".download.*")), [])
        self.assertIn("--proto-redir\n=https", (self.repo / "commands").read_text())


if __name__ == "__main__":
    unittest.main()
