#!/usr/bin/env python3
"""Consolidate terminal stage records, including billed usage reported before interruptions."""
import json
from run_benchmark import ROOT, events, result_usage, save

def main():
    environment=json.loads((ROOT/'run-environment.json').read_text())
    all_runs=[]
    for case in environment['cases']:
        arm=[]
        for stage in [1,2,3]:
            prefix=f'{case}-stage-{stage}'
            path=ROOT/f'{prefix}-run.json'
            if not path.exists():continue
            record=json.loads(path.read_text())
            assert record['status']!='running',prefix+' is still running'
            rows=events(ROOT/f'{prefix}-events.jsonl')
            initial=record.get('recovery_of_initial_timeout')
            if initial:
                rows=events(ROOT/f'{prefix}-initial-events.jsonl')+rows
            if record['harness']=='smith':
                deltas=[r['event']['payload']['record']['delta'] for r in rows if
                    r.get('type')=='runtime_event' and r['event'].get('payload',{}).get('event')=='usage' and
                    r['event']['payload']['record']['source']=='provider_attempt']
                usage={key:sum(d.get(key,0) for d in deltas)for key in ['input_cached','input_uncached','output','reasoning']}
                usage['provider_attempts']=sum(r.get('event',{}).get('payload',{}).get('event')=='provider_attempt_started' for r in rows)
                usage['provider_attempts_with_reported_usage']=len(deltas)
                record.setdefault('terminal_usage_projection',record.get('usage'))
                record['usage']=usage
            total=record['usage']['input_cached']+record['usage']['input_uncached']
            record['cached_input_percent']=round(100*record['usage']['input_cached']/total,2)if total else None
            record['usage_scope']='Provider-reported usage; sums archived timeout and recovery events where applicable. Unreported timeout usage is unknown.'
            save(path,record);arm.append(record)
        save(ROOT/f'{case}-runs.json',arm)
        all_runs+=arm
    save(ROOT/'runs.json',all_runs)
    checks=[]
    for stage in [1,2,3]:
        expected_prompt=__import__('hashlib').sha256((ROOT/f'prompt-{stage}.txt').read_bytes()).hexdigest()
        observations=[]
        for record in [r for r in all_runs if r['stage']==stage]:
            source=record.get('recovery_of_initial_timeout',record)
            observations.append({'case':record['case'],'initial_prompt_matches':source['prompt_sha256']==expected_prompt,
                                 'initial_images':source['images'],'same_session_resume':stage==1 or record['session_id']==record['resumed_session']})
        checks.append({'stage':stage,'records':observations,
                       'initial_image_hashes_equal_across_arms':len({json.dumps(r['initial_images'],sort_keys=True)for r in observations})==1})
    save(ROOT/'input-parity.json',checks)
    print(json.dumps([{k:r[k]for k in ['case','stage','status','cached_input_percent','elapsed_seconds']}for r in all_runs],indent=2))

if __name__=='__main__':main()
