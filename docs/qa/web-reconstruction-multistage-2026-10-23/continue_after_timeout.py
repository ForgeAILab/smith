#!/usr/bin/env python3
"""Continue to the next requested checkpoint from an interrupted prior stage."""
import asyncio
import json
from pathlib import Path
import shutil
import tempfile
from run_benchmark import ROOT, events, run_case, save

async def main():
    env=json.loads((ROOT/'run-environment.json').read_text())
    record=json.loads((ROOT/'smith-sol-stage-2-run.json').read_text())
    assert record['status']=='timeout'
    rows=events(ROOT/'smith-sol-stage-2-events.jsonl')
    sid=next(r['event']['session']for r in rows if r.get('type')=='runtime_event')
    old_root=Path(env['scratch'])/'sessions'
    old=next(old_root.rglob(sid+'.snapshot.json')).parent
    new_root=Path(tempfile.mkdtemp(prefix='smith-web-local-sessions-',dir='/private/tmp'))
    shutil.copytree(old,new_root/old.name)
    change={'case':'smith-sol','after_stage':2,'reason':'Initial stage-2 wall-clock limit; updated dashboard already written. Continue original stage-3 task without grader feedback.',
            'session_id':sid,'project_path_unchanged':record['project'],'old_sessions_directory':str(old_root),'new_sessions_directory':str(new_root),
            'stage3_timeout_seconds':900}
    save(ROOT/'storage-correction.json',change)
    env.update(starting_stage=3,initial_session=sid,sessions_directory=str(new_root),timeout_seconds_per_stage=900)
    await run_case('smith-sol',env)

if __name__=='__main__':asyncio.run(main())
