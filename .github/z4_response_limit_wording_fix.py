from pathlib import Path

p = Path("docs/Z4_SECURITY_BOUNDARIES.md")
s = p.read_text()
old = "and zero or larger explicit values fail closed before request-ID allocation, pending-slot use or request-body-budget consumption."
new = "and zero or over-maximum explicit values fail closed before request-ID allocation, pending-slot use or request-body-budget consumption."
if s.count(old) != 1:
    raise SystemExit(f"expected one wording match, got {s.count(old)}")
p.write_text(s.replace(old, new, 1))
