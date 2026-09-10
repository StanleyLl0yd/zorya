from pathlib import Path

driver = Path("scripts/z3_115_driver.py")
exec(compile(driver.read_text(encoding="utf-8"), str(driver), "exec"), {"__name__": "__main__"})

module = Path("src/profile_runtime/session_restore_persistence.rs")
text = module.read_text(encoding="utf-8")
simple = ".ok_or_else(|| {"
multiline = ".ok_or_else(\n            || {"
if text.count(simple) != 3:
    raise SystemExit(f"expected 3 simple ok_or_else sites, found {text.count(simple)}")
if text.count(multiline) != 1:
    raise SystemExit(f"expected 1 multiline ok_or_else site, found {text.count(multiline)}")
text = text.replace(simple, ".ok_or({")
text = text.replace(multiline, ".ok_or(\n            {")
module.write_text(text, encoding="utf-8")
