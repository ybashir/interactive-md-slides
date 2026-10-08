#!/usr/bin/env python3
"""Preserve notices for dependencies actually compiled into the API binary."""
import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
manifest = root / "services/app/Cargo.toml"
registry = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "registry/src"
tree = subprocess.check_output(
    ["cargo", "tree", "--locked", "--manifest-path", str(manifest), "--edges", "normal,build", "--prefix", "none"],
    text=True,
)
packages = sorted(set(re.findall(r"^(\S+) v([^\s]+)", tree, re.MULTILINE)))
groups = {}
for name, version in packages:
    if name == "interdeck-app":
        continue
    candidates = list(registry.glob(f"*/{name}-{version}"))
    if len(candidates) != 1:
        raise RuntimeError(f"Cannot locate pinned source for {name}@{version}")
    source = candidates[0]
    files = [path for path in source.iterdir() if path.is_file() and re.match(r"^(license|licence|copying|copyright|notice)([._-]|$)", path.name, re.I)]
    for directory in ("LICENSES", "licenses"):
        if (source / directory).is_dir():
            files.extend(path for path in (source / directory).iterdir() if path.is_file())
    if not files:
        raise RuntimeError(f"Missing license notices for {name}@{version}")
    notice = "\n\n".join(f"{path.relative_to(source)}\n{path.read_text()}" for path in sorted(files))
    digest = hashlib.sha256(notice.encode()).hexdigest()
    group = groups.setdefault(digest, {"packages": [], "notice": notice})
    group["packages"].append(f"{name}@{version}")
output = Path(sys.argv[1])
output.write_text("Rust API dependency notices\n\n" + "\n\n--------------------\n\n".join(
    ", ".join(group["packages"]) + "\n\n" + group["notice"] for group in groups.values()
))
print(f"Preserved notices for {len(packages) - 1} compiled Rust dependencies")
