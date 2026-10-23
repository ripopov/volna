import argparse
import importlib.util
import json
import os
import platform
import posixpath
import re
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

EXT = Path(__file__).resolve().parent
ROOT = EXT.parent
spec = importlib.util.spec_from_file_location(
    "distribution_notices", ROOT / "volna/tools/distribution_notices.py"
)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)
MEDIA = (
    "webview.mjs",
    "webview.css",
    "theme.mjs",
    "volna.js",
    "volna_bg.wasm",
    "design/styles.css",
)
IMPORT = re.compile(
    r"""(?:\bfrom\s+|\b(?:import|require)\s*(?:\(\s*)?)["'](\.[^"']+)["']"""
)
CSS_IMPORT = re.compile(r"""@import\s+["']([^"']+)["']|url\(["']?([^\s"')]+)["']?\)""")


def check_files(read, roots, prefix):
    pending = [posixpath.normpath(root) for root in roots]
    checked = set()
    while pending:
        item = pending.pop()
        if item in checked:
            continue
        checked.add(item)
        try:
            data = read(item)
        except (KeyError, FileNotFoundError) as error:
            raise ValueError(
                f"missing {prefix}{item}; run npm run build first"
            ) from error
        if not data:
            raise ValueError(f"empty {prefix}{item}")
        if item.endswith((".js", ".mjs", ".cjs")):
            dependencies = IMPORT.findall(data.decode("utf-8"))
        elif item.endswith(".css"):
            dependencies = [
                first or second
                for first, second in CSS_IMPORT.findall(data.decode("utf-8"))
            ]
        else:
            continue
        for dependency in dependencies:
            if dependency.startswith("data:"):
                continue
            target = posixpath.normpath(
                posixpath.join(posixpath.dirname(item), dependency)
            )
            if target.startswith(("../", "/")) or ":" in target:
                raise ValueError(
                    f"{prefix}{item} imports outside package: {dependency}"
                )
            if item.endswith((".js", ".cjs")) and not posixpath.splitext(target)[1]:
                target += ".js"
            pending.append(target)


def check_media(read):
    check_files(read, MEDIA, "media/")


def host_target():
    system = platform.system()
    machine = platform.machine().lower()
    arch = {"x86_64": "x64", "amd64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(
        machine
    )
    os_name = {"Linux": "linux", "Darwin": "darwin", "Windows": "win32"}.get(system)
    if not os_name or not arch:
        raise ValueError(f"unsupported packaging host: {system} {machine}")
    if os_name == "linux" and platform.libc_ver()[0] != "glibc":
        raise ValueError("Linux packaging requires glibc; Alpine needs its own target")
    return f"{os_name}-{arch}"


def check_binary(data, target):
    os_name, arch = target.split("-")
    if os_name == "linux":
        if len(data) < 20 or data[:6] != b"\x7fELF\x02\x01":
            raise ValueError("expected a 64-bit little-endian ELF executable")
        actual = struct.unpack_from("<H", data, 18)[0]
        expected = {"x64": 62, "arm64": 183}[arch]
    elif os_name == "darwin":
        if len(data) < 8 or data[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError("expected a 64-bit little-endian Mach-O executable")
        actual = struct.unpack_from("<I", data, 4)[0]
        expected = {"x64": 0x01000007, "arm64": 0x0100000C}[arch]
    else:
        if len(data) < 64 or data[:2] != b"MZ":
            raise ValueError("expected a Windows PE executable")
        offset = struct.unpack_from("<I", data, 0x3C)[0]
        if offset + 6 > len(data) or data[offset : offset + 4] != b"PE\0\0":
            raise ValueError("invalid Windows PE header")
        actual = struct.unpack_from("<H", data, offset + 4)[0]
        expected = {"x64": 0x8664, "arm64": 0xAA64}[arch]
    if actual != expected:
        raise ValueError(
            f"executable architecture {actual:#x} does not match {target} ({expected:#x})"
        )


def verify_vsix(vsix, target, binaries):
    with zipfile.ZipFile(vsix) as archive:
        files = set(archive.namelist())
        manifest = ET.fromstring(archive.read("extension.vsixmanifest"))
        ns = {"v": "http://schemas.microsoft.com/developer/vsx-schema/2011"}
        identity = manifest.find("v:Metadata/v:Identity", ns)
        if identity is None or identity.get("TargetPlatform") != target:
            raise ValueError(f"VSIX targetPlatform is not {target}")
        package = json.loads(archive.read("extension/package.json"))
        if identity.get("Version") != package["version"]:
            raise ValueError("VSIX version does not match package.json")
        suffix = ".exe" if target.startswith("win32-") else ""
        expected = {f"extension/bin/{name}{suffix}" for name in ("volna-server", "vtr")}
        bundled = {
            item
            for item in files
            if item.startswith("extension/bin/") and not item.endswith("/")
        }
        if bundled != expected:
            raise ValueError(f"VSIX contains unexpected executables: {sorted(bundled)}")
        for name in ("volna-server", "vtr"):
            item = f"extension/bin/{name}{suffix}"
            binary = archive.read(item)
            check_binary(binary, target)
            if binary != binaries[name].read_bytes():
                raise ValueError(f"VSIX {name} differs from the native build")
            if not suffix and not (archive.getinfo(item).external_attr >> 16 & 0o111):
                raise ValueError(f"VSIX {name} is not executable")
        check_files(
            lambda item: archive.read(f"extension/{item}"), [package["main"]], ""
        )
        check_media(lambda item: archive.read(f"extension/media/{item}"))
        notices.verify(lambda item: archive.read(f"extension/licenses/{item}"))


def package(target):
    check_media(lambda item: (EXT / "media" / item).read_bytes())
    binaries = {}
    for manifest, package_name in (
        (ROOT / "Cargo.toml", "volna-server"),
        (ROOT / "vtr/Cargo.toml", "vtr-cli"),
    ):
        result = subprocess.run(
            [
                "cargo", "build", "--locked", "--manifest-path", str(manifest),
                "--target-dir", str(ROOT / "target"), "-p", package_name,
                "--release", "--message-format=json-render-diagnostics",
            ],
            cwd=ROOT,
            check=True,
            stdout=subprocess.PIPE,
            text=True,
        )
        for line in result.stdout.splitlines():
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message.get("executable"):
                name = message["target"]["name"]
                if name in ("volna-server", "vtr"):
                    binaries[name] = Path(message["executable"])
    if binaries.keys() != {"volna-server", "vtr"}:
        raise ValueError("Cargo did not report both native executables")
    bin_dir = EXT / "bin"
    bin_dir.mkdir(exist_ok=True)
    suffix = ".exe" if target.startswith("win32-") else ""
    for name, built in binaries.items():
        check_binary(built.read_bytes(), target)
        help_result = subprocess.run(
            [str(built), "--help"], capture_output=True, text=True, check=True, timeout=10
        )
        if f"Usage: {name}" not in help_result.stdout:
            raise ValueError(f"native {name} did not start correctly")
        with tempfile.NamedTemporaryFile(
            dir=bin_dir, prefix=f".{name}-", delete=False
        ) as temp:
            temp_path = Path(temp.name)
        try:
            shutil.copy2(built, temp_path)
            os.replace(temp_path, bin_dir / f"{name}{suffix}")
        finally:
            temp_path.unlink(missing_ok=True)
        other_suffix = "" if suffix else ".exe"
        (bin_dir / f"{name}{other_suffix}").unlink(missing_ok=True)
    notices.collect(EXT / "licenses", {"volna", "volna-server", "vtr-cli"})
    version = json.loads((EXT / "package.json").read_text())["version"]
    vsix = EXT / f"volna-{version}-{target}.vsix"
    shutil.copyfile(ROOT / "LICENSE", EXT / "LICENSE")
    node = shutil.which("node")
    command = [node, str(EXT / "node_modules/@vscode/vsce/vsce")]
    subprocess.run(
        [
            *command,
            "package",
            "--no-dependencies",
            "--target",
            target,
            "--out",
            str(vsix),
        ],
        cwd=EXT,
        check=True,
    )
    verify_vsix(vsix, target, binaries)
    print(f"Verified {vsix}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Build and verify a VS Code package for the host platform."
    )
    parser.add_argument(
        "--target", help="expected VS Code target; must match this build host"
    )
    args = parser.parse_args()
    actual_target = host_target()
    if args.target and args.target != actual_target:
        parser.error(f"{args.target} does not match the build host {actual_target}")
    package(actual_target)
