#!/usr/bin/env python3
"""Resume an interrupted benchmark stage without adding references or grading feedback."""
import argparse
import asyncio
import json
import shutil
from run_benchmark import ROOT, events, run_case, save

async def main(args):
    environment=json.loads((ROOT/'run-environment.json').read_text())
    prefix=f'{args.case}-stage-{args.stage}'
    initial=json.loads((ROOT/f'{prefix}-run.json').read_text())
    assert initial['status']=='timeout'
    rows=events(ROOT/f'{prefix}-events.jsonl')
    session=next(r['event']['session'] for r in rows if r.get('type')=='runtime_event')
    usage={'input_cached':0,'input_uncached':0,'output':0}
    for row in rows:
        payload=row.get('event',{}).get('payload',{})
        if payload.get('event')=='usage' and payload['record']['source']=='provider_attempt':
            for key in usage:usage[key]+=payload['record']['delta'].get(key,0)
    initial.update(usage=usage,session_id=session)
    for suffix in ['-run.json','-events.jsonl','.stderr']:
        shutil.copyfile(ROOT/(prefix+suffix),ROOT/(prefix+'-initial'+suffix))
    save(ROOT/f'{prefix}-initial-run.json',initial)
    environment.update(recovery_session=session,initial_timeout=initial,starting_stage=args.stage,
        recovery_stage=args.stage,recovery_prompt=f'prompt-recovery-{args.stage}.txt',timeout_seconds_per_stage=900)
    await run_case(args.case,environment)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case',required=True)
    parser.add_argument('--stage',required=True,type=int)
    asyncio.run(main(parser.parse_args()))
