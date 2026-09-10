from pathlib import Path

path = Path("src/session_restore.rs")
text = path.read_text(encoding="utf-8")
old = '''    fn raw_record(\n        generation: u64,\n        schema: u32,\n        next_window_id: u64,\n        next_tab_id: u64,\n        windows: &[(u64, u64, &[(u64, &str)])],\n    ) -> Vec<u8> {'''
new = '''    type RawTab<'a> = (u64, &'a str);\n    type RawWindow<'a> = (u64, u64, &'a [RawTab<'a>]);\n\n    fn raw_record(\n        generation: u64,\n        schema: u32,\n        next_window_id: u64,\n        next_tab_id: u64,\n        windows: &[RawWindow<'_>],\n    ) -> Vec<u8> {'''
if text.count(old) != 1:
    raise SystemExit("expected raw_record signature exactly once")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
