from pathlib import Path

driver = Path("scripts/z3_115_driver.py")
exec(compile(driver.read_text(encoding="utf-8"), str(driver), "exec"), {"__name__": "__main__"})

module = Path("src/profile_runtime/session_restore_persistence.rs")
text = module.read_text(encoding="utf-8")n
