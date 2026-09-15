from pathlib import Path
import gzip, base64, hashlib
p = Path('.github/smith-resume-review.patch.gz.b64')
s = p.read_text()
assert hashlib.sha256(s.encode()).hexdigest() == '95702a2e4b607d6947cf52e6bf5106b2a7371dcf203d1b4b51060a4d944b5b0b'
repairs = [(1725, 1727, 'R')]
for start, end, replacement in reversed(repairs):
    s = s[:start] + replacement + s[end:]
assert hashlib.sha256(gzip.decompress(base64.b64decode(s))).hexdigest() == '52c9782f09ab8889aab23c10f7ae98569ad97f76d05652d1a21fcb06fd1ea9f8'
p.write_text(s)
