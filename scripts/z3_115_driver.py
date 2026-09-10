from pathlib import Path

source_path = Path("scripts/z3_115_session_persistence.py")
source = source_path.read_text(encoding="utf-8")
old = r'''replace_once(
    "src/profile_runtime.rs",
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n        &self.session_restore\n    }\n\n    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n        self.session_restore_recovery.as_ref()\n    }",
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n"
    "        self.session_restore.snapshot()\n"
    "    }\n\n"
    "    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n"
    "        self.session_restore.recovery()\n"
    "    }",
)'''
new = r'''replace_once(
    "src/profile_runtime.rs",
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n        self.bookmarks.recovery()\n    }\n\n    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n        &self.session_restore\n    }\n\n    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n        self.session_restore_recovery.as_ref()\n    }",
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n"
    "        self.bookmarks.recovery()\n"
    "    }\n\n"
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n"
    "        self.session_restore.snapshot()\n"
    "    }\n\n"
    "    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n"
    "        self.session_restore.recovery()\n"
    "    }",
)'''
if source.count(old) != 1:
    raise SystemExit("could not find the exact ambiguous session accessor staging block")
source = source.replace(old, new, 1)
exec(compile(source, str(source_path), "exec"), {"__name__": "__main__"})
