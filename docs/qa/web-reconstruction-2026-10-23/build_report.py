#!/usr/bin/env python3
"""Build an offline comparison viewer from measured benchmark evidence."""

import html
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
NAMES = {'smith-sol':'Smith / Sol', 'codex-sol':'Codex CLI / Sol',
         'smith-astra':'Smith / Astra', 'codex-astra':'Codex CLI / Astra'}


def main():
    runs = {r['case']:r for r in json.loads((ROOT/'runs.json').read_text())}
    evaluation = json.loads((ROOT/'evaluation.json').read_text())
    usage = {row['case']:row for row in json.loads((ROOT/'summary.json').read_text())['rows']}
    order=['smith-sol','codex-sol','smith-astra','codex-astra']
    results = sorted(evaluation['results'],key=lambda result:order.index(result['case']))
    rows, cards = [], []
    for result in results:
        case=result['case']; run=runs[case]
        if result['status']!='rendered':
            rows.append(f'<tr><th>{NAMES[case]}</th><td colspan="6">No renderable output: {html.escape(run["status"])}</td></tr>')
            continue
        checks=result['interactions']; passed=sum(bool(checks[k]) for k in ['new_booking_dialog_opens','escape_closes_dialog','booking_submission_adds_row','chart_period_changes_state','sidebar_selection_changes'])
        mobile=next(x for x in result['responsive'] if x['width']==375)
        rows.append(f'''<tr><th scope="row">{NAMES[case]}</th><td>{run['elapsed_seconds']:.0f}s</td><td>{result['text_anchors_matched']}/{result['text_anchors_total']}</td><td>{result['mean_region_iou']:.3f}</td><td>{result['pixel_metrics']['ssim']:.3f}</td><td>{usage[case]['cached_input_percent']:.1f}%</td><td>{passed}/5</td><td>{'Overflow' if mobile['horizontalOverflow'] else 'Fits'}</td></tr>''')
        cards.append(f'''<article class="card"><div class="card-title"><h3>{NAMES[case]}</h3><a href="outputs/{case}/index.html">Open implementation ↗</a></div><a href="screenshots/{case}-desktop.png"><img loading="lazy" width="1536" height="1024" src="screenshots/{case}-desktop.png" alt="Unedited desktop browser capture from {NAMES[case]}"></a><p><a href="screenshots/{case}-768.png">Tablet capture</a> · <a href="screenshots/{case}-375.png">Mobile capture</a></p></article>''')
    options=''.join(f'<option value="{r["case"]}">{NAMES[r["case"]]}</option>' for r in results if r['status']=='rendered')
    page='''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="description" content="Measured screenshot reconstruction comparison of Smith and Codex CLI using GPT-6.1 Sol and GPT-6 Astra."><title>Smith × Codex screenshot benchmark</title>
<style>
:root{--paper:#f5f4ef;--panel:#fff;--ink:#102f25;--muted:#5e6c65;--line:#d4ddd6;--accent:#d9fa8a;--s1:4px;--s2:8px;--s3:12px;--s4:16px;--s6:24px;--s8:32px;--radius:12px;--body:16px;--small:14px;--h2:22px;--display:36px}
*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:var(--body)/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Arial,sans-serif}main{max-width:1480px;margin:auto;padding:var(--s8) var(--s6)}h1{font-size:var(--display);line-height:1.15;letter-spacing:-.03em;margin:var(--s2) 0 var(--s4)}h2{font-size:var(--h2);margin:0 0 var(--s4)}h3{font-size:var(--body);margin:0}p{margin:var(--s2) 0 var(--s4)}.eyebrow,.muted{color:var(--muted)}.eyebrow{font-size:var(--small);font-weight:600;letter-spacing:.08em;text-transform:uppercase}.panel{padding:var(--s6);background:var(--panel);border:1px solid var(--line);border-radius:var(--radius);margin-top:var(--s6)}a{color:var(--ink);text-underline-offset:3px}a:hover{background:var(--accent)}button,select{min-height:44px;padding:var(--s2) var(--s4);font:inherit;border:1px solid var(--line);border-radius:var(--s2);color:var(--ink);background:var(--panel)}button{cursor:pointer}button:hover,button[aria-pressed="true"]{background:var(--accent)}button:active{transform:translateY(1px)}button:disabled{opacity:.5;cursor:default}:focus-visible{outline:3px solid var(--ink);outline-offset:3px}.toolbar{display:flex;align-items:center;gap:var(--s3);flex-wrap:wrap;margin-bottom:var(--s4)}.toolbar label{font-size:var(--small);font-weight:600}.modes{display:flex;gap:var(--s2);flex-wrap:wrap}input[type=range]{accent-color:var(--ink);min-width:160px;min-height:44px;flex:1}.stage{position:relative;aspect-ratio:3/2;background:var(--paper);border:1px solid var(--line);overflow:hidden}.stage img{position:absolute;inset:0;width:100%;height:100%;object-fit:contain}.stage #actual{opacity:.5}.table-wrap{overflow:auto}table{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums;white-space:nowrap}th,td{padding:var(--s3) var(--s4);border-bottom:1px solid var(--line);text-align:right}th:first-child,td:first-child{text-align:left}thead th{color:var(--muted);font-size:var(--small)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:var(--s6);margin-top:var(--s6)}.card{background:var(--panel);border:1px solid var(--line);border-radius:var(--radius);overflow:hidden}.card-title{display:flex;align-items:center;justify-content:space-between;gap:var(--s3);padding:var(--s4)}.card-title a,.card p{font-size:var(--small)}.card img{display:block;width:100%;height:auto}.card p{padding:0 var(--s4)}summary{cursor:pointer;min-height:44px;font-weight:600}li{margin:var(--s2) 0}.footer{font-size:var(--small);margin-top:var(--s6)}@media(max-width:700px){main{padding:var(--s6) var(--s4)}h1{font-size:var(--h2)}.panel{padding:var(--s4)}.grid{grid-template-columns:1fr}.toolbar{align-items:stretch}.toolbar select{width:100%}.card-title{align-items:flex-start;flex-direction:column}.modes button{flex:1}.modes{width:100%}}
</style></head><body><main>
<header><div class="eyebrow">Visual reconstruction experiment · October 23, 2026</div><h1>One screenshot. Four builds.</h1><p>GPT-6.1 Sol and GPT-6 Astra, each tested through Smith and Codex CLI. Same screenshot, task, reasoning effort and ten-minute limit. Every implementation is preserved as produced.</p></header>
<section class="panel" aria-labelledby="comparison-heading"><h2 id="comparison-heading">Inspect the difference</h2><div class="toolbar"><label for="case">Reconstruction</label><select id="case">OPTIONS</select><div class="modes" aria-label="Comparison mode"><button type="button" data-mode="reference" aria-pressed="false">Reference</button><button type="button" data-mode="actual" aria-pressed="false">Reconstruction</button><button type="button" data-mode="overlay" aria-pressed="true">Overlay</button></div><label for="opacity">Reconstruction opacity</label><input id="opacity" type="range" min="0" max="100" value="50"></div><div class="stage"><img width="1536" height="1024" src="reference.png" alt="Generated WAYFINDER dashboard reference"><img id="actual" width="1536" height="1024" alt="Selected reconstruction"></div><p class="muted">Change opacity to inspect alignment, spacing, typography and chart geometry. Screenshots use the same 1536×1024 viewport.</p></section>
<section class="panel" aria-labelledby="metrics-heading"><h2 id="metrics-heading">Measured evidence</h2><div class="table-wrap"><table><thead><tr><th scope="col">Harness / model</th><th scope="col">Elapsed</th><th scope="col">Text anchors</th><th scope="col">Region IoU</th><th scope="col">SSIM</th><th scope="col">Cached input</th><th scope="col">Interactions</th><th scope="col">375px</th></tr></thead><tbody>ROWS</tbody></table></div><p class="muted">Higher region IoU and SSIM mean closer geometric and pixel agreement. Neither is an overall product-quality score. Cache percentages use provider-reported input totals; native prompts and tool sequences differ between harnesses.</p><details><summary>Method and limitations</summary><ul><li>One generated reference, one attempt per arm. The marker and metric values were not supplied as text.</li><li>Text coverage checks visible DOM text against a manually transcribed set of anchors. Region IoU uses nine manually measured reference regions and the matching DOM containers.</li><li>SSIM compares original-size RGB browser captures. Fonts, antialiasing and small shifts affect the number; the full images remain the primary visual evidence.</li><li>Interaction checks cover opening the booking dialog, Escape dismissal, adding a booking row, chart-period selection and sidebar selection. Failures include the grader's error notes in evaluation.json.</li><li>Runs used each harness's native tools and system instructions. Two balanced waves ran two arms concurrently. Elapsed time includes initialization and tool use and is not a controlled latency ranking.</li><li>Desktop, tablet and mobile were rendered in real Chrome. These are unedited benchmark outputs, so detected defects are reported rather than repaired.</li></ul><p><a href="prompt.txt">Exact task</a> · <a href="ground-truth.json">Ground truth</a> · <a href="evaluation.json">Full browser evaluation</a> · <a href="runs.json">Run records</a></p></details></section>
<section class="grid" aria-label="All unedited reconstructions">CARDS</section><p class="footer muted">Local benchmark artifact. No page was published and no runtime behavior was changed.</p>
</main><script>
const picker=document.querySelector('#case'),actual=document.querySelector('#actual'),slider=document.querySelector('#opacity');
function choose(){actual.src='screenshots/'+picker.value+'-desktop.png';actual.alt=picker.options[picker.selectedIndex].text+' desktop reconstruction';}
function opacity(){actual.style.opacity=slider.value/100;document.querySelectorAll('[data-mode]').forEach(b=>b.setAttribute('aria-pressed',String(b.dataset.mode==='overlay')));}
picker.addEventListener('change',choose);slider.addEventListener('input',opacity);
document.querySelectorAll('[data-mode]').forEach(button=>button.addEventListener('click',()=>{const mode=button.dataset.mode;slider.value=mode==='reference'?0:mode==='actual'?100:50;actual.style.opacity=slider.value/100;document.querySelectorAll('[data-mode]').forEach(b=>b.setAttribute('aria-pressed',String(b===button)));}));choose();
</script></body></html>'''
    page=page.replace('OPTIONS',options).replace('ROWS',''.join(rows)).replace('CARDS',''.join(cards))
    (ROOT/'comparison.html').write_text(page)


if __name__=='__main__':
    main()
