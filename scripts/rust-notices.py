"""Collect Windows Cargo dependency notices without publishing local cache paths."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("metadata", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--repository", type=Path, required=True)
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding="utf-8-sig"))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    pending = [p["id"] for p in packages.values() if p["name"] == "diskburrow-app"]
    selected = set()
    while pending:
        key = pending.pop()
        if key in selected:
            continue
        selected.add(key)
        for dependency in nodes[key]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    # Cargo's resolved feature graph can include test-enabled features. Including
    # their notices conservatively is preferable to claiming a minimal SBOM.
    args.destination.mkdir(exist_ok=False, parents=True)
    names = re.compile(r"(?i)(licen[sc]e|copying|notice|copyright|^ofl)")
    records = []
    missing = []
    for key in sorted(selected):
        package = packages[key]
        root = Path(package["manifest_path"]).parent
        texts = [p for p in root.rglob("*") if p.is_file()
                 and names.search(p.name) and p.suffix.lower() not in
                 {".rs", ".c", ".h", ".svg", ".py", ".yml", ".json"}]
        if not texts and package["source"] is None:
            texts = [args.repository / "LICENSE"]
        fallback = args.repository / "rust" / "licenses" / (package["name"] + ".txt")
        if fallback.exists():
            texts.append(fallback)
        if not texts:
            missing.append(package["name"] + " " + package["version"])
        folder = args.destination / (package["name"] + "-" + package["version"])
        folder.mkdir()
        copied = []
        for number, path in enumerate(sorted(set(texts))):
            relative = path.relative_to(root).as_posix() if path.is_relative_to(root) else path.name
            target = folder / (str(number) + "-" + path.name)
            shutil.copyfile(path, target)
            copied.append({"source_name": relative,
                           "file": target.relative_to(args.destination).as_posix(),
                           "sha256": hashlib.sha256(target.read_bytes()).hexdigest()})
        records.append({"name": package["name"], "version": package["version"],
                        "license": package["license"], "authors": package["authors"],
                        "repository": package["repository"],
                        "source_archive": ("https://crates.io/api/v1/crates/" +
                                           package["name"] + "/" + package["version"] + "/download")
                        if package["source"] else None, "notices": copied})
    if missing:
        raise SystemExit("Missing license texts: " + ", ".join(missing))
    (args.destination / "index.json").write_text(
        json.dumps({"scope": "Conservative Windows normal/build resolved Cargo graph; includes some test-unified features",
                    "packages": records}, indent=2, ensure_ascii=False), encoding="utf-8")
    print("Collected notices for", len(records), "packages")


if __name__ == "__main__":
    main()
