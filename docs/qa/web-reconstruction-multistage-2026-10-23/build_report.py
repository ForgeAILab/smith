#!/usr/bin/env python3
"""Build an offline viewer and compact machine-readable benchmark summary."""
import json
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT=Path(__file__).resolve().parent

def main():
    environment=json.loads((ROOT/'run-environment.json').read_text())
    evaluation=json.loads((ROOT/'evaluation.json').read_text())
    runs=json.loads((ROOT/'runs.json').read_text())
    summaries=[]
    for case in environment['cases']:
        turns=sorted([r for r in runs if r['case']==case],key=lambda r:r['stage'])
        screens=[r for r in evaluation['results'] if r['case']==case]
        total_cached=sum(r['usage']['input_cached'] for r in turns)
        total_uncached=sum(r['usage']['input_uncached'] for r in turns)
        summaries.append({'case':case,'completed_stages':sum(r['status']=='completed' for r in turns),
            'session_id':turns[0]['session_id'],'same_session_all_stages':len({r['session_id'] for r in turns})==1,
            'cached_input':total_cached,'uncached_input':total_uncached,
            'cached_input_percent':round(100*total_cached/(total_cached+total_uncached),2) if total_cached+total_uncached else None,
            'elapsed_seconds':round(sum(r['elapsed_seconds'] for r in turns),2),
            'turns':[{k:r[k] for k in ['stage','status','elapsed_seconds','cached_input_percent','usage','recovery_of_initial_timeout'] if k in r} for r in turns],
            'screens':[{k:r[k] for k in ['stage','page','text_anchors_matched','text_anchors_total','marker_visible','observation_marker_correct','pixel_metrics','responsive','missing_text','superseded_text_visible','critical_values_visible'] if k in r} for r in screens],
            'dashboard_preservation':next((r for r in evaluation['dashboard_preservation'] if r['case']==case),None)})
    payload={'environment':environment,'cases':summaries,'evaluation':evaluation,'runs':runs,
             'limitations':['One attempt per arm, no repeated trials or blinded human preference scores.',
                'Image slices and references are generated fixtures; 6 supplied attachments across 3 turns, not a 192-image stress test.',
                'Pixel similarity is descriptive and sensitive to typography/antialiasing; image understanding is measured separately.',
                'Four concurrent arms share the host and backend; elapsed time is not a controlled latency ranking. Smith persisted sessions under /Volumes/Data/tmp while Codex used ~/.codex/sessions; a passive Smith sample caught sync_all waiting. Storage is a timing confound.',
                'Smith/Sol stage 1 and Smith/Astra stage 3 reached initial 600-second limits and received documented same-session recoveries. Smith/Sol stages 2 and 3 produced their files but reached their 600/900-second limits before a final answer. Stage 3 used the same session with local session storage. External browser checks passed on all produced pages. Original logs, usage and extra elapsed time are retained.',
                'Smith uses the fixed experimental ChatGPT serializer. Codex uses installed CLI 0.163.0, not the fetched source worktree.']}
    (ROOT/'summary.json').write_text(json.dumps(payload,indent=2,ensure_ascii=False)+'\n')
    specs=[(1,'index.html','original.png','1 · Combine three image parts'),(2,'index.html','revision.png','2 · Revise the dashboard'),(3,'bookings.html','bookings.png','3 · Add Bookings'),(3,'index.html','revision.png','3 · Preserve the revised dashboard')]
    # A compact contact sheet for each checkpoint, with one reference and four arms.
    font_path='/System/Library/Fonts/Supplemental/Arial.ttf'
    font=ImageFont.truetype(font_path,23)
    for stage,page,ref,title in specs:
        canvas=Image.new('RGB',(1560,1194),'#edf0e9');draw=ImageDraw.Draw(canvas)
        draw.text((20,12),title,fill='#173a2e',font=font)
        tiles=[('Reference',ROOT/'references'/ref)]
        tiles += [(case,ROOT/'screenshots'/f'{case}-stage-{stage}-{Path(page).stem}-desktop.png') for case in environment['cases']]
        for i,(label,path) in enumerate(tiles):
            x=12+(i%3)*520;y=60+(i//3)*562
            draw.text((x,y),label,fill='#173a2e',font=font)
            if path.exists():
                image=Image.open(path).convert('RGB');image.thumbnail((508,508))
                canvas.paste(image,(x,y+40))
        canvas.save(ROOT/f'contact-stage-{stage}-{Path(page).stem}.png')
    template=(ROOT/'viewer-template.html').read_text()
    (ROOT/'comparison.html').write_text(template.replace('__DATA__',json.dumps(payload,ensure_ascii=False).replace('</','<\\/')))
    report=['# Three-stage image reconstruction benchmark',
        '\n[Open the interactive comparison](comparison.html). Frozen model outputs, real Chrome screenshots, observations and raw event logs are included.\n',
        'Each arm combines three horizontal screenshot parts into a dashboard, resumes to apply a revised screenshot, then resumes again to add a Bookings page while retaining the revised dashboard. Models are gpt-6.1-sol and gpt-6-astra at medium effort, through Smith and installed Codex CLI 0.163.0.\n',
        '| Arm | Build SSIM | Revision SSIM | Bookings SSIM | Dashboard retained | Cached input |',
        '|---|---:|---:|---:|---|---:|']
    for c in summaries:
        def screen(stage,page):return next((s for s in c['screens'] if s['stage']==stage and s['page']==page),{})
        def score(stage,page):
            s=screen(stage,page)
            return f"{s['pixel_metrics']['ssim']:.3f}" if s.get('pixel_metrics')else'—'
        retained=c.get('dashboard_preservation',{});retained=retained.get('revision_retained')if retained else None
        report.append(f"| {c['case']} | {score(1,'index.html')} | {score(2,'index.html')} | {score(3,'bookings.html')} | {'Yes' if retained else 'No' if retained is False else 'Not completed'} | {c['cached_input_percent']:.2f}% |")
    report+=['\n## Evidence and limits\n',
        '- [evaluation.json](evaluation.json) separates visible text, image identifiers, observations, revised values, superseded content, navigation, filters, booking forms, browser errors and mobile overflow.',
        '- [image-history-audit.json](image-history-audit.json) hashes persisted inputs, checks source-byte equality and verifies all six supplied images remain in the correct order. It is not a capture of every outgoing native request.',
        '- [input-parity.json](input-parity.json) checks initial prompt and image hashes across arms. Recovery prompts add no visual information or grading feedback.',
        '- [runs.json](runs.json) includes provider-reported cached and uncached input. Timed attempts and recoveries are combined; any unreported usage from interrupted requests remains unknown.',
        '- Smith uses the frozen binary with two native serialization fixes described in the [cache investigation](../cache-prefix-investigation-2026-10-23/REPORT.md). The remaining miss pattern also reproduces without images.',
        '- Initial stage limits were 600 seconds. Smith/Sol stage 1 hit a provider stream timeout and was recovered in the same session. Smith/Astra stage 3 reached the wall-clock limit and was recovered from its existing images and observations. Those initial attempts are preserved in `*-initial-*` files. Smith/Sol stage 2 wrote the correct revision but reached its limit before a final answer; its original stage-3 prompt then continued the same session with storage copied to `/private/tmp` and a 900-second limit. Stage 3 also reached its limit after writing both pages. External browser checks passed on the produced pages. Thus 10/12 logical stages received a terminal model answer, while all 16 requested page snapshots were produced and rendered. See `storage-correction.json`.',
        '- Elapsed times are confounded by concurrent execution and storage: Smith sessions were under `/Volumes/Data/tmp`, while Codex used its default session directory. A [passive sample](profiler-smith-sol-stage-2.txt) caught Smith waiting in `sync_all`. This is not a model-speed ranking. Future timing runs must place all state explicitly on the same local volume.',
        '- This is one attempt per arm with documented interruption recoveries, not a statistical model ranking. Pixel SSIM is sensitive to typography and antialiasing and is separate from image comprehension.',
        '- Six supplied image attachments across three stages do not reproduce the original 192-image session or its image-window eviction.\n',
        'The model outputs are unmodified. Browser checks operate on frozen copies; no evaluator feedback was sent to the models. Small decorations are reconstructed in CSS/SVG, not taken from the reference images.']
    (ROOT/'REPORT.md').write_text('\n'.join(report)+'\n')
    print(json.dumps(summaries,indent=2))

if __name__=='__main__':main()
