"""Build a native Volna distribution with its matching VTR converter and notices."""

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from tools import distribution_notices

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists():
        parser.error(f"output already exists: {output}")
    target = args.target_dir.resolve()
    command = [
        "cargo", "build", "--locked", "--target-dir", str(target),
        "-p", "volna", "-p", "vtr-cli", "--bins",
    ]
    if not args.debug:
        command.append("--release")
    subprocess.run(command, cwd=ROOT, check=True)
    profile = "debug" if args.debug else "release"
    suffix = ".exe" if os.name == "nt" else ""
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="volna-package-", dir=output.parent) as temporary:
        staged = Path(temporary) / "volna"
        staged.mkdir()
        for name in ("volna", "vtr"):
            shutil.copy2(target / profile / f"{name}{suffix}", staged)
        distribution_notices.collect(staged / "notices", {"volna", "vtr-cli"})
        distribution_notices.verify(lambda name: (staged / "notices" / name).read_bytes())
        (staged / "README.txt").write_text(
            "Run volna with a VTR, FST, Kanata, or saved .volna.json workspace.\n"
            "Keep vtr beside volna: it converts FST and Kanata imports automatically.\n"
            "Dependency licenses are in notices/.\n",
            encoding="utf-8",
        )
        staged.rename(output)
    print(output)


if __name__ == "__main__":
    main()
