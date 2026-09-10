from pathlib import Path

source_path = Path("scripts/z3_117_session_native.py")
source = source_path.read_text()
old = '''release = replace_once(release, package_anchor, package_block + package_anchor, "packaged session smoke")
release_path.write_text(release)
'''
new = '''if release.count(package_anchor) != 2:
    raise SystemExit(
        f"packaged session smoke: expected two bookmark-toggle anchors, found {release.count(package_anchor)}"
    )
before, separator, after = release.rpartition(package_anchor)
release = before + package_block + separator + after
release_path.write_text(release)
'''
if source.count(old) != 1:
    raise SystemExit("staging-driver source patch did not match exactly once")
source = source.replace(old, new, 1)
exec(compile(source, str(source_path), "exec"))
