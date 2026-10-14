import hashlib
import json
import shutil
import subprocess
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATIC = {
    "LICENSE": ROOT / "LICENSE",
    "fonts/Inter-OFL.txt": ROOT / "volna-core/assets/fonts/Inter-OFL.txt",
    "fonts/JetBrainsMono-OFL.txt": ROOT
    / "volna-core/assets/fonts/JetBrainsMono-OFL.txt",
    "fonts/IBMPlexSans-LICENSE.txt": ROOT
    / "volna-core/assets/fonts/IBMPlexSans-LICENSE.txt",
    "icons/Lucide-LICENSE.txt": ROOT / "volna-core/assets/icons/LICENSE",
}


def collect(destination, packages):
    destination = Path(destination)
    if destination.exists():
        shutil.rmtree(destination)
    for name, source in STATIC.items():
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=ROOT,
            timeout=300,
        )
    )
    by_id = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    pending = [p["id"] for p in by_id.values() if p["name"] in packages]
    seen = set()
    while pending:
        item = pending.pop()
        if item in seen:
            continue
        seen.add(item)
        pending.extend(
            d["pkg"]
            for d in nodes[item]["deps"]
            if any(k["kind"] != "dev" for k in d["dep_kinds"])
        )
    inventory = []
    for item in sorted(seen):
        if item in metadata["workspace_members"]:
            continue
        package = by_id[item]
        source = Path(package["manifest_path"]).parent
        key = f"{package['name']}-{package['version']}"
        folder = destination / "dependencies" / key
        files = sorted(
            p
            for p in source.rglob("*")
            if p.is_file()
            and p.name.lower().startswith(
                ("license", "licence", "copying", "notice", "copyright")
            )
        )
        if package.get("license_file"):
            files = sorted(set(files + [source / package["license_file"]]))
        if not files:
            folder.mkdir(parents=True, exist_ok=True)
            expression = package.get("license") or ""
            if "Apache-2.0" in expression and "AND" not in expression:
                shutil.copyfile(
                    ROOT / "volna/tools/licenses/Apache-2.0.txt",
                    folder / "LICENSE-APACHE",
                )
            elif expression in ("MPL-2.0", "CC0-1.0"):
                shutil.copyfile(
                    ROOT / f"volna/tools/licenses/{expression}.txt",
                    folder / f"LICENSE-{expression}",
                )
            elif expression == "MIT":
                terms = (ROOT / "LICENSE").read_text().split("Permission", 1)[1]
                (folder / "LICENSE-MIT").write_text(
                    "MIT License\n\nCopyright: see accompanying upstream source notices.\n\nPermission"
                    + terms
                )
            else:
                raise ValueError(f"{key}: missing upstream license text ({expression})")
            with tarfile.open(folder / "upstream-source.tar.gz", "w:gz") as archive:
                archive.add(source, arcname=key)
        for file in files:
            target = folder / file.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(file, target)
        inventory.append(
            {
                k: package.get(k)
                for k in (
                    "name",
                    "version",
                    "license",
                    "repository",
                    "authors",
                    "source",
                )
            }
        )
    (destination / "DEPENDENCIES.json").write_text(
        json.dumps(inventory, indent=2) + "\n"
    )
    checksums = {
        str(p.relative_to(destination)).replace("\\", "/"): hashlib.sha256(
            p.read_bytes()
        ).hexdigest()
        for p in sorted(destination.rglob("*"))
        if p.is_file()
    }
    (destination / "FILES.json").write_text(json.dumps(checksums, indent=2) + "\n")


def verify(read):
    for name, source in STATIC.items():
        if read(name) != source.read_bytes():
            raise ValueError(f"incorrect license notice: {name}")
    inventory = json.loads(read("DEPENDENCIES.json"))
    checksums = json.loads(read("FILES.json"))
    if not inventory or not checksums:
        raise ValueError("empty dependency notices")
    for package in inventory:
        prefix = f"dependencies/{package['name']}-{package['version']}/"
        if not any(name.startswith(prefix) for name in checksums):
            raise ValueError(f"missing dependency notices: {prefix}")
    for name, digest in checksums.items():
        if name.startswith("/") or ".." in Path(name).parts:
            raise ValueError(f"invalid notice path: {name}")
        if hashlib.sha256(read(name)).hexdigest() != digest:
            raise ValueError(f"incorrect dependency notice: {name}")
