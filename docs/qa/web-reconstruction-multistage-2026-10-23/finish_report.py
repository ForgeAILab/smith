#!/usr/bin/env python3
"""Wait for the final arm, then consolidate evidence and verify the viewer."""
import base64
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT=Path(__file__).resolve().parent

def run(command):
    print(json.dumps({'phase':'running','command':command}),flush=True)
    subprocess.run(command,check=True,cwd=ROOT)

def compliance():
    env=json.loads((ROOT/'run-environment.json').read_text())
    references={r['sha256']:name for name,r in env['image_metadata'].items()}
    records=[]
    for case in env['cases']:
        for stage in [1,2,3]:
            folder=ROOT/'outputs'/case/f'stage-{stage}'
            copied=[];embedded=[]
            for path in folder.rglob('*'):
                if not path.is_file():continue
                if hashlib.sha256(path.read_bytes()).hexdigest()in references:copied.append(str(path.relative_to(folder)))
                if path.suffix in ['.html','.css','.js']:
                    for encoded in re.findall(r'data:image/[^;,]+;base64,([A-Za-z0-9+/=]+)',path.read_text()):
                        digest=hashlib.sha256(base64.b64decode(encoded)).hexdigest()
                        if digest in references:embedded.append({'file':str(path.relative_to(folder)),'reference':references[digest]})
            records.append({'case':case,'stage':stage,'copied_reference_files':copied,'embedded_reference_data':embedded})
    (ROOT/'source-compliance.json').write_text(json.dumps(records,indent=2)+'\n')
    assert not any(r['copied_reference_files']or r['embedded_reference_data']for r in records)

def main():
    path=ROOT/'smith-sol-stage-3-run.json'
    while json.loads(path.read_text())['status']=='running':time.sleep(2)
    run([sys.executable,'finalize_runs.py'])
    run(['uv','run','--with','pillow','python','audit_images.py'])
    compliance()
    run(['uv','run','--with','playwright','--with','pillow','--with','numpy','--with','scikit-image','python','evaluate.py'])
    run(['uv','run','--with','pillow','python','build_report.py'])
    run(['uv','run','--with','playwright','python','verify_viewer.py'])
    print(json.dumps({'phase':'complete','viewer':str(ROOT/'comparison.html')}),flush=True)

if __name__=='__main__':main()
