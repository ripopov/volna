import importlib.util
import json
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "package_vsix", Path(__file__).with_name("package_vsix.py")
)
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


def binary(target):
    os_name, arch = target.split("-")
    if os_name == "linux":
        data = bytearray(64)
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", data, 18, {"x64": 62, "arm64": 183}[arch])
    elif os_name == "darwin":
        data = bytearray(64)
        data[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", data, 4, {"x64": 0x01000007, "arm64": 0x0100000C}[arch])
    else:
        data = bytearray(128)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 80)
        data[80:84] = b"PE\0\0"
        struct.pack_into("<H", data, 84, {"x64": 0x8664, "arm64": 0xAA64}[arch])
    return bytes(data)


class PackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.notices_temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.notices_temp.cleanup)
        cls.notices_dir = Path(cls.notices_temp.name)
        packager.notices.collect(cls.notices_dir, {"volna", "volna-server", "vtr-cli"})

    def add_notices(self, archive):
        for file in self.notices_dir.rglob("*"):
            if file.is_file():
                archive.write(
                    file,
                    "extension/licenses/"
                    + file.relative_to(self.notices_dir).as_posix(),
                )

    def test_css_and_host_dependencies_must_be_packaged(self):
        files = {
            "styles.css": b'@import "tokens/fonts.css";',
            "tokens/fonts.css": b'@font-face {src: url("../fonts/Inter.woff2");}',
        }
        with self.assertRaisesRegex(ValueError, "missing media/fonts/Inter.woff2"):
            packager.check_files(files.__getitem__, ["styles.css"], "media/")
        files["fonts/Inter.woff2"] = b"font"
        packager.check_files(files.__getitem__, ["styles.css"], "media/")
        files = {"extension.js": b'const host = require("./workspace");'}
        with self.assertRaisesRegex(ValueError, "missing workspace.js"):
            packager.check_files(files.__getitem__, ["extension.js"], "")
        files["workspace.js"] = b"module.exports = {};"
        packager.check_files(files.__getitem__, ["extension.js"], "")
        files["workspace.js"] = b'require("../outside");'
        with self.assertRaisesRegex(ValueError, "outside package"):
            packager.check_files(files.__getitem__, ["extension.js"], "")

    def test_binary_architectures(self):
        for os_name in ("linux", "darwin", "win32"):
            for arch in ("x64", "arm64"):
                target = f"{os_name}-{arch}"
                with self.subTest(target=target):
                    packager.check_binary(binary(target), target)
                    wrong = f"{os_name}-{'arm64' if arch == 'x64' else 'x64'}"
                    with self.assertRaisesRegex(ValueError, "does not match"):
                        packager.check_binary(binary(target), wrong)

    def test_vsix_rejects_missing_wrong_or_mismatched_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "volna-server"
            server.write_bytes(binary("linux-x64"))
            converter = root / "vtr"
            converter.write_bytes(binary("linux-x64") + b"converter")
            binaries = {"volna-server": server, "vtr": converter}
            vsix = root / "volna.vsix"

            def write_vsix(
                target, bundled, notices=True, converter_bytes=None,
                include_converter=True, executable=True,
            ):
                identity = f' TargetPlatform="{target}"' if target else ""
                manifest = f'<PackageManifest xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"><Metadata><Identity Version="0.1.0"{identity}/></Metadata></PackageManifest>'
                with zipfile.ZipFile(vsix, "w") as archive:
                    if notices:
                        self.add_notices(archive)
                    archive.writestr("extension.vsixmanifest", manifest)
                    archive.writestr(
                        "extension/package.json",
                        json.dumps({"version": "0.1.0", "main": "./extension.js"}),
                    )
                    archive.writestr("extension/extension.js", b"host")
                    executables = {"volna-server": bundled}
                    if include_converter:
                        executables["vtr"] = (
                            converter.read_bytes() if converter_bytes is None
                            else converter_bytes
                        )
                    for name, data in executables.items():
                        info = zipfile.ZipInfo(f"extension/bin/{name}")
                        info.external_attr = (0o100755 if executable else 0o100644) << 16
                        archive.writestr(info, data)
                    for item in packager.MEDIA:
                        archive.writestr(f"extension/media/{item}", b"content")

            write_vsix(None, server.read_bytes())
            with self.assertRaisesRegex(ValueError, "targetPlatform"):
                packager.verify_vsix(vsix, "linux-x64", binaries)
            write_vsix("linux-x64", binary("linux-arm64"))
            with self.assertRaisesRegex(ValueError, "does not match"):
                packager.verify_vsix(vsix, "linux-x64", binaries)
            write_vsix("linux-x64", server.read_bytes())
            packager.verify_vsix(vsix, "linux-x64", binaries)
            for converter_bytes, include_converter, executable, error in (
                (None, False, True, "unexpected executables"),
                (binary("linux-arm64"), True, True, "does not match"),
                (binary("linux-x64"), True, True, "vtr differs"),
                (None, True, False, "not executable"),
            ):
                write_vsix(
                    "linux-x64", server.read_bytes(), converter_bytes=converter_bytes,
                    include_converter=include_converter, executable=executable,
                )
                with self.assertRaisesRegex(ValueError, error):
                    packager.verify_vsix(vsix, "linux-x64", binaries)
            write_vsix("linux-x64", server.read_bytes(), notices=False)
            with self.assertRaises(KeyError):
                packager.verify_vsix(vsix, "linux-x64", binaries)

    def test_notices_reject_missing_files(self):
        files = {
            p.relative_to(self.notices_dir).as_posix(): p.read_bytes()
            for p in self.notices_dir.rglob("*")
            if p.is_file()
        }
        for name in (
            *packager.notices.STATIC,
            "DEPENDENCIES.json",
            "FILES.json",
            next(n for n in files if n.startswith("dependencies/")),
        ):
            with self.subTest(name=name):
                missing = dict(files)
                del missing[name]
                with self.assertRaises(KeyError):
                    packager.notices.verify(missing.__getitem__)

    def test_vsix_rejects_missing_snippet_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "volna-server"
            server.write_bytes(binary("linux-x64"))
            converter = root / "vtr"
            converter.write_bytes(binary("linux-x64") + b"converter")
            binaries = {"volna-server": server, "vtr": converter}
            vsix = root / "volna.vsix"
            with zipfile.ZipFile(vsix, "w") as archive:
                self.add_notices(archive)
                archive.writestr(
                    "extension.vsixmanifest",
                    '<PackageManifest xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"><Metadata><Identity Version="0.1.1" TargetPlatform="linux-x64"/></Metadata></PackageManifest>',
                )
                archive.writestr(
                    "extension/package.json",
                    json.dumps({"version": "0.1.1", "main": "./extension.js"}),
                )
                archive.writestr("extension/extension.js", b"host")
                for name, built in binaries.items():
                    info = zipfile.ZipInfo(f"extension/bin/{name}")
                    info.external_attr = 0o100755 << 16
                    archive.writestr(info, built.read_bytes())
                for item in ("webview.mjs", "webview.css", "design/styles.css"):
                    archive.writestr(f"extension/media/{item}", b"content")
                archive.writestr(
                    "extension/media/theme.mjs", b"export const theme = {};"
                )
                archive.writestr("extension/media/volna_bg.wasm", b"wasm")
                archive.writestr(
                    "extension/media/volna.js",
                    "import { copyTableText } from './snippets/generated/src/table_clipboard.mjs';",
                )
            with self.assertRaisesRegex(
                ValueError, "missing media/snippets/generated/src/table_clipboard.mjs"
            ):
                packager.verify_vsix(vsix, "linux-x64", binaries)
            with zipfile.ZipFile(vsix, "a") as archive:
                archive.writestr(
                    "extension/media/snippets/generated/src/table_clipboard.mjs",
                    "export { copyTableText } from '../copy.mjs';",
                )
            with self.assertRaisesRegex(
                ValueError, "missing media/snippets/generated/copy.mjs"
            ):
                packager.verify_vsix(vsix, "linux-x64", binaries)
            with zipfile.ZipFile(vsix, "a") as archive:
                archive.writestr(
                    "extension/media/snippets/generated/copy.mjs",
                    "export function copyTableText() {}",
                )
            packager.verify_vsix(vsix, "linux-x64", binaries)


if __name__ == "__main__":
    unittest.main()
