#!/usr/bin/env python3
"""One documented recovery of the provider timeout; same images, files and session."""
import asyncio
import json
import shutil
from run_benchmark import ROOT, events, run_case, save

async def main():
    environment=json.loads((ROOT/'run-environment.json').read_text())
    initial=json.loads((ROOT/'smith-sol-stage-1-run.json').read_text())
    assert initial['status']=='timeout'
    rows=events(ROOT/'smith-sol-stage-1-events.jsonl')
    session=next(r['event']['session'] for r in rows if r.get('type')=='runtime_event')
    usage={'input_cached':0,'input_uncached':0,'output':0}
    for row in rows:
        payload=row.get('event',{}).get('payload',{})
        if payload.get('event')=='usage' and payload['record']['source']=='provider_attempt':
            for key in usage:usage[key]+=payload['record']['delta'].get(key,0)
    initial['usage']=usage
    initial['session_id']=session
    for suffix in ['run.json','events.jsonl','stderr']:
        source=ROOT/f'smith-sol-stage-1-{suffix}' if suffix!='stderr' else ROOT/'smith-sol-stage-1.stderr'
        target=ROOT/f'smith-sol-stage-1-initial-{suffix}' if suffix!='stderr' else ROOT/'smith-sol-stage-1-initial.stderr'
        shutil.copyfile(source,target)
    save(ROOT/'smith-sol-stage-1-initial-run.json',initial)
    environment.update(recovery_session=session,initial_timeout=initial)
    await run_case('smith-sol',environment)

if __name__=='__main__':asyncio.run(main())
