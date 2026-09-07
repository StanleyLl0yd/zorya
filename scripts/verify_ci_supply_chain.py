#!/usr/bin/env python3
import re
from pathlib import Path

ROOTS = (Path(".github/workflows"), Path(".github/actions"))
ACTION_REF = re.compile(r"^\s*(?:-\s*)?uses:\s*([^\s#]+)")
IMAGE = re.compile(r"^\s*image:\s*([^\s#]+)")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"@sha256:[0-9a-f]{64}$")
TOP_LEVEL_PERMISSIONS = re.compile(r"(?m)^permissions:\s*(?:\{\}|$)")

errors = []

for root in ROOTS:
    if not root.exists():
        continue

    for path in sorted(root.rglob("*")):
        if path.suffix not in {".yml", ".yaml"}:
            continue

        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()

        if "pull_request_target:" in text:
            errors.append(f"{path}: pull_request_target is forbidden")
        if "secrets: inherit" in text:
            errors.append(f"{path}: secrets inheritance is forbidden")
        if "write-all" in text:
            errors.append(f"{path}: write-all permissions are forbidden")
        if path.parent == Path(".github/workflows") and not TOP_LEVEL_PERMISSIONS.search(text):
            errors.append(f"{path}: explicit top-level workflow permissions are required")

        for number, line in enumerate(lines, start=1):
            action = ACTION_REF.match(line)
            if action:
                target = action.group(1)

                if target.startswith("./"):
                    continue

                if target.startswith("docker://"):
                    image = target.removeprefix("docker://")
                    if not DIGEST.search(image):
                        errors.append(
                            f"{path}:{number}: docker action image must be pinned by sha256 digest"
                        )
                    continue

                if "@" not in target:
                    errors.append(f"{path}:{number}: action is not pinned")
                    continue

                owner_action, ref = target.rsplit("@", 1)
                if not owner_action or not FULL_SHA.fullmatch(ref):
                    errors.append(
                        f"{path}:{number}: action ref must use a full 40-character commit SHA"
                    )

                if target.startswith("actions/checkout@"):
                    nearby = "\n".join(lines[number : number + 10])
                    if "persist-credentials: false" not in nearby:
                        errors.append(
                            f"{path}:{number}: checkout must set persist-credentials: false"
                        )

            image = IMAGE.match(line)
            if image:
                target = image.group(1)
                if target.startswith("${{"):
                    continue
                if not DIGEST.search(target):
                    errors.append(
                        f"{path}:{number}: container image must be pinned by sha256 digest"
                    )

if errors:
    raise SystemExit("\n".join(errors))

print("GitHub Actions supply-chain policy: OK")
