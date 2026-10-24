#!/usr/bin/env python3
"""Three image-driven turns per persistent session, four independent arms."""
import asyncio
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import time

ROOT = Path(__file__).resolve().parent
IMAGES = {
    1: ['part-1.png', 'part-2.png', 'part-3.png'],
    2: ['revision.png'],
    3: ['bookings.png', 'revision.png'],
}

def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def events(path):
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return rows

def result_usage(harness, rows):
    if harness == 'smith':
        result = next((r for r in reversed(rows) if r.get('type') == 'result'), {})
        usage = result.get('usage', {}).get('current_turn', {})
        return result.get('session_id'), {
            'input_cached': usage.get('input_cached', 0),
            'input_uncached': usage.get('input_uncached', 0),
            'output': usage.get('output', 0),
            'provider_attempts': result.get('lifecycle', {}).get('attempts_committed'),
        }
    sid = next((r['thread_id'] for r in rows if r.get('type') == 'thread.started'), None)
    usage = next((r.get('usage', {}) for r in reversed(rows) if r.get('type') == 'turn.completed'), {})
    return sid, {
        'input_cached': usage.get('cached_input_tokens', 0),
        'input_uncached': usage.get('input_tokens', 0) - usage.get('cached_input_tokens', 0),
        'output': usage.get('output_tokens', 0),
        'command_calls': sum(r.get('type') == 'item.completed' and r.get('item', {}).get('type') == 'command_execution' for r in rows),
    }

async def run_case(label, environment):
    project = Path(environment['scratch']) / label
    project.mkdir(parents=True, exist_ok=True)
    harness, model_label = label.split('-', 1)
    model = 'gpt-6.1-sol' if model_label == 'sol' else 'gpt-6-astra'
    (project / 'AGENTS.md').write_text(
        'This directory is an isolated screenshot reconstruction benchmark. Work only here, '
        'do not inspect parent/sibling directories, do not delegate. Use self-contained vanilla '
        'HTML/CSS/JavaScript. Reference images are inspection-only and must not become page assets. '
        'Write DESIGN.md inferred tokens before UI code; retain and update it across turns.\n')
    env = {**os.environ, 'TMPDIR': '/private/tmp'}
    if harness == 'smith':
        (project / '.smith').mkdir(exist_ok=True)
        (project / '.smith/config.toml').write_text(f'''profile_order = ["web-benchmark"]
[profiles.web-benchmark]
provider = "chatgpt"
model = "{model}"
posture = "build"
use = ["main"]
delegation = false
advisor = false
vision = false
max_output_tokens = 16384
''')
        env.update({'SMITH_PERSISTENCE_SESSIONS_DIR': environment.get('sessions_directory',str(Path(environment['scratch'])/'sessions')),
                    'SMITH_CONTEXT_CACHE_MAINTENANCE': 'off', 'SMITH_CONTEXT_CACHE_IDLE_COMPACTION': 'false'})
    first_stage=environment.get('starting_stage',1)
    records=json.loads((ROOT/f'{label}-runs.json').read_text()) if first_stage>1 and (ROOT/f'{label}-runs.json').exists() else []
    records=[r for r in records if r['stage']<first_stage]
    session = environment.get('recovery_session',environment.get('initial_session'))
    for stage in range(first_stage,4):
        for name in IMAGES[stage]:
            shutil.copyfile(ROOT / 'references' / name, project / name)
        prompt = (ROOT / f'prompt-{stage}.txt').read_text()
        image_names=IMAGES[stage]
        recovering=stage==environment.get('recovery_stage',1) and bool(environment.get('recovery_session'))
        if recovering:
            prompt=(ROOT/environment.get('recovery_prompt','prompt-recovery.txt')).read_text()
            image_names=[]
        if harness == 'smith':
            command = [environment['smith_binary'], '--project', str(project), '--profile', 'web-benchmark',
                       '--effort', 'medium', '--approval', 'allow-all', '--output-format', 'stream-json']
            if session:
                command += ['--resume', session]
            for name in image_names:
                command += ['--image', str(project/name)]
            command += ['-p', '-']
        else:
            command = [environment['codex_binary'], 'exec', '--ignore-user-config', '--ignore-rules',
                       '--skip-git-repo-check', '--cd', str(project), '--sandbox', 'workspace-write',
                       '-c', 'approval_policy="never"', '-c', 'model_reasoning_effort="medium"']
            if session:
                command += ['resume', session]
            command += ['--model', model]
            for name in image_names:
                command += ['--image', str(project/name)]
            command += ['--json', '-']
        prefix = f'{label}-stage-{stage}'
        record = {'case':label, 'stage':stage, 'model':model, 'harness':harness, 'command':command,
                  'project':str(project), 'resumed_session':session, 'started_at_epoch':time.time(),
                  'status':'running', 'prompt_sha256':hashlib.sha256(prompt.encode()).hexdigest(),
                  'images':[{ 'name':n, 'sha256':sha(project/n)} for n in image_names]}
        if harness=='smith':record['session_storage']=env['SMITH_PERSISTENCE_SESSIONS_DIR']
        if recovering:
            record['recovery_of_initial_timeout']=environment['initial_timeout']
        save(ROOT/f'{prefix}-run.json', record)
        print(json.dumps({'case':label,'stage':stage,'status':'started','resume':session}), flush=True)
        started = time.monotonic()
        log = ROOT/f'{prefix}-events.jsonl'
        with log.open('wb') as out, (ROOT/f'{prefix}.stderr').open('wb') as err:
            process = await asyncio.create_subprocess_exec(*command,stdin=asyncio.subprocess.PIPE,
                stdout=out,stderr=err,cwd=project,env=env,start_new_session=True)
            try:
                await asyncio.wait_for(process.communicate(prompt.encode()), environment['timeout_seconds_per_stage'])
                record.update(status='completed' if process.returncode == 0 else 'failed',exit_code=process.returncode)
            except asyncio.TimeoutError:
                os.killpg(process.pid,signal.SIGKILL)
                await process.wait()
                record.update(status='timeout',exit_code=process.returncode)
        record['elapsed_seconds'] = round(time.monotonic()-started,2)
        returned_session, usage = result_usage(harness,events(log))
        session = returned_session or session
        record['session_id'] = session
        record['usage'] = usage
        if recovering:
            initial=environment['initial_timeout']
            record['recovery_elapsed_seconds']=record['elapsed_seconds']
            record['elapsed_seconds']+=initial['elapsed_seconds']
            for key in ['input_cached','input_uncached','output']:
                usage[key]+=initial['usage'].get(key,0)
        total = usage['input_cached']+usage['input_uncached']
        record['cached_input_percent'] = round(100*usage['input_cached']/total,2) if total else None
        target = ROOT/'outputs'/label/f'stage-{stage}'
        target.mkdir(parents=True,exist_ok=True)
        excluded = set(sum(IMAGES.values(),[])) | {'AGENTS.md'}
        record['generated_files'] = []
        for path in project.rglob('*'):
            rel = path.relative_to(project)
            if path.is_file() and rel.parts[0] not in ['.smith','.git'] and rel.name not in excluded:
                dest=target/rel
                dest.parent.mkdir(parents=True,exist_ok=True)
                shutil.copyfile(path,dest)
                record['generated_files'].append({'path':str(rel),'sha256':sha(path)})
        save(ROOT/f'{prefix}-run.json',record)
        records.append(record)
        save(ROOT/f'{label}-runs.json',records)
        print(json.dumps({k:record[k] for k in ['case','stage','status','elapsed_seconds','session_id','cached_input_percent']}),flush=True)
        if record['status'] != 'completed' or not session:
            break
    return records

async def main():
    environment=json.loads((ROOT/'run-environment.json').read_text())
    results=await asyncio.gather(*(run_case(label,environment) for label in environment['cases']))
    save(ROOT/'runs.json',[record for arm in results for record in arm])

if __name__=='__main__':
    asyncio.run(main())
