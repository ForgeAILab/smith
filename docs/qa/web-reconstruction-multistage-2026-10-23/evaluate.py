#!/usr/bin/env python3
"""Render immutable stage snapshots in Chrome; never repair benchmark outputs."""
import asyncio
import functools
import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import re
import threading

import numpy as np
from PIL import Image, ImageFilter
from playwright.async_api import async_playwright
from skimage.metrics import structural_similarity

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('single_image_evaluator', ROOT.parent/'web-reconstruction-2026-10-23/evaluate.py')
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)
contains, normalize = common.contains, common.normalize

def save(name,value):
    (ROOT/name).write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n')

def source_hash(folder):
    digest=hashlib.sha256()
    for path in sorted(folder.rglob('*')):
        if path.is_file():
            digest.update(str(path.relative_to(folder)).encode());digest.update(path.read_bytes())
    return digest.hexdigest()

def metrics(reference,actual):
    a,b=[Image.open(p).convert('RGB') for p in [reference,actual]]
    return {'ssim':round(float(structural_similarity(np.asarray(a),np.asarray(b),channel_axis=2,data_range=255)),5),
            'blurred_ssim_radius_3':round(float(structural_similarity(np.asarray(a.filter(ImageFilter.GaussianBlur(3))),np.asarray(b.filter(ImageFilter.GaussianBlur(3))),channel_axis=2,data_range=255)),5)}

def observed_text(value):
    if isinstance(value,dict):
        return ' '.join(observed_text(v) for v in value.values())
    if isinstance(value,list):
        return ' '.join(observed_text(v) for v in value)
    return '' if value is None else str(value)

async def visible_booking_rows(page):
    return await page.locator('table tbody tr:visible').evaluate_all('''rows=>rows.map(row=>{
        const walker=document.createTreeWalker(row,NodeFilter.SHOW_TEXT),parts=[];let node;
        while((node=walker.nextNode())){
            const parent=node.parentElement;
            if(parent&&parent.getClientRects().length&&getComputedStyle(parent).visibility!=='hidden')parts.push(node.textContent);
        }
        return parts.join(' ');
    })''')

async def booking_interactions(page,url):
    result={'confirmed_filter':False,'pending_filter':False,'all_filter_restores_rows':False,
            'search_matches_member':False,'search_matches_space':False,'search_is_case_insensitive':False,'notes':[]}
    async def rows():
        return await visible_booking_rows(page)
    initial=await rows()
    for label,key in [('Confirmed','confirmed_filter'),('Pending','pending_filter')]:
        try:
            button=page.get_by_role('button',name=re.compile('^'+label+'(?:\\s*\\d+)?$',re.I)).first
            if not await button.count():
                button=page.get_by_role('tab',name=re.compile('^'+label,re.I)).first
            await button.click(timeout=1800)
            await page.wait_for_timeout(100)
            values=await rows()
            result[key]=bool(values) and len(values)<len(initial) and all(contains(v,label) for v in values)
        except Exception as error:
            result['notes'].append(key+': '+str(error).splitlines()[0])
        await page.goto(url,wait_until='networkidle')
    try:
        pending=page.get_by_role('button',name=re.compile('^Pending(?:\\s*\\d+)?$',re.I)).first
        if not await pending.count():pending=page.get_by_role('tab',name=re.compile('^Pending',re.I)).first
        await pending.click(timeout=1800)
        all_tab=page.get_by_role('button',name=re.compile('^All bookings',re.I)).first
        if not await all_tab.count():all_tab=page.get_by_role('tab',name=re.compile('^All bookings',re.I)).first
        await all_tab.click(timeout=1800)
        result['all_filter_restores_rows']=len(await rows())==len(initial)
    except Exception as error:
        result['notes'].append('restore: '+str(error).splitlines()[0])
    await page.goto(url,wait_until='networkidle')
    try:
        field=page.get_by_placeholder(re.compile('search bookings',re.I)).first
        for query,key,expected in [('Maya','search_matches_member','Maya Patel'),('Studio 01','search_matches_space','Lucas Brown'),('mAyA','search_is_case_insensitive','Maya Patel')]:
            await field.fill(query)
            await page.wait_for_timeout(100)
            values=await rows()
            result[key]=len(values)==1 and contains(values[0],expected)
        await field.fill('')
    except Exception as error:
        result['notes'].append('search: '+str(error).splitlines()[0])
    return result

async def booking_dialog(page):
    result={'new_booking_dialog_opens':False,'escape_closes_dialog':False,'booking_submission_adds_row':False,'notes':[]}
    try:
        button=page.get_by_role('button',name=re.compile('new booking',re.I)).first
        await button.click(timeout=1800)
        dialog=page.locator('dialog[open], [role="dialog"]:visible').first
        result['new_booking_dialog_opens']=await dialog.is_visible()
        await page.keyboard.press('Escape')
        result['escape_closes_dialog']=not await dialog.is_visible()
        await button.click(timeout=1800)
        fields=dialog.locator('input:not([type="hidden"]), textarea, select')
        for i in range(await fields.count()):
            field=fields.nth(i);tag,kind=await field.evaluate('(e)=>[e.tagName,e.type]')
            if tag=='SELECT':
                options=await field.locator('option').evaluate_all('(a)=>a.filter(e=>e.value&&!e.disabled).map(e=>e.value)')
                if options:await field.select_option(options[0])
            elif kind in ['text','search','textarea','tel']:await field.fill('Benchmark Guest')
            elif kind=='email':await field.fill('benchmark@example.test')
            elif kind=='date':await field.fill('2026-10-24')
            elif kind=='time':await field.fill('10:00')
            elif kind=='datetime-local':await field.fill('2026-10-24T10:00')
            elif kind=='number':await field.fill('1')
        submit=dialog.locator('button[type="submit"], input[type="submit"]')
        if not await submit.count():submit=dialog.get_by_role('button',name=re.compile('create|save|add|confirm|book',re.I))
        await submit.last.click(timeout=1800)
        await page.wait_for_timeout(100)
        text=' '.join(await visible_booking_rows(page))
        if not contains(text,'Benchmark Guest'):
            search=page.get_by_placeholder(re.compile('search bookings',re.I)).first
            if await search.count():
                await search.fill('Benchmark Guest')
                text=' '.join(await visible_booking_rows(page))
        result['booking_submission_adds_row']=contains(text,'Benchmark Guest')
    except Exception as error:
        result['notes'].append(str(error).splitlines()[0])
    return result

async def render(browser,port,label,stage,page_name,truth):
    folder=ROOT/'outputs'/label/f'stage-{stage}'
    html=folder/page_name
    if not html.exists():
        observation=None
        try:observation=json.loads((folder/f'observations-{stage}.json').read_text())
        except (OSError,json.JSONDecodeError):pass
        return {'case':label,'stage':stage,'page':page_name,'status':'missing','observation':observation,
                'observation_marker_correct':contains(observed_text(observation),truth['recognition_marker'])}
    before=hashlib.sha256(html.read_bytes()).hexdigest()
    viewport={'width':1536,'height':1024}
    context=await browser.new_context(viewport=viewport,device_scale_factor=1,reduced_motion='reduce')
    page=await context.new_page()
    errors,external,failed=[],[],[]
    page.on('pageerror',lambda error:errors.append(str(error)))
    page.on('response',lambda response:failed.append({'url':response.url,'status':response.status}) if response.status>=400 else None)
    async def route(request):
        if request.request.url.startswith(f'http://127.0.0.1:{port}/') or request.request.url.startswith('data:'):
            await request.continue_()
        else:
            external.append(request.request.url)
            await request.abort()
    await page.route('**/*',route)
    url=f'http://127.0.0.1:{port}/outputs/{label}/stage-{stage}/{page_name}'
    await page.goto(url,wait_until='networkidle')
    await page.evaluate('document.fonts.ready')
    await page.wait_for_timeout(200)
    stem=f'{label}-stage-{stage}-{Path(page_name).stem}'
    shots=ROOT/'screenshots';shots.mkdir(exist_ok=True)
    desktop=shots/f'{stem}-desktop.png'
    await page.screenshot(path=str(desktop),animations='disabled')
    dom=await common.dom_evidence(page)
    placeholders=await page.locator('input[placeholder]:visible').evaluate_all('(a)=>a.map(e=>e.placeholder).join(" ")')
    dom['visibleText']+=' '+placeholders
    anchors=truth['text_anchors']
    missing=[a for a in anchors if not contains(dom['visibleText'],a)]
    observation_stage=2 if stage==3 and page_name=='index.html' else stage
    observation_file=folder/f'observations-{observation_stage}.json'
    observation=None
    try:observation=json.loads(observation_file.read_text())
    except (OSError,json.JSONDecodeError):pass
    obs=observed_text(observation)
    result={'case':label,'stage':stage,'page':page_name,'status':'rendered',
            'text_anchors_matched':len(anchors)-len(missing),'text_anchors_total':len(anchors),
            'missing_text':missing,'marker_visible':contains(dom['visibleText'],truth['recognition_marker']),
            'pixel_metrics':metrics(ROOT/'references'/truth['reference'],desktop),
            'desktop_dom':{k:v for k,v in dom.items() if k!='elements'},
            'observation':observation,'observation_marker_correct':contains(obs,truth['recognition_marker']),
            'observation_metrics':{v:contains(obs,v) for v in truth['critical_values']},
            'critical_values_visible':{v:contains(dom['visibleText'],v) for v in truth['critical_values']},
            'superseded_text_visible':[v for v in truth.get('superseded_text',[]) if contains(dom['visibleText'],v)],
            'responsive':[],'screenshots':{'desktop':str(desktop.relative_to(ROOT))}}
    for width in [768,375]:
        await page.set_viewport_size({'width':width,'height':1024})
        await page.wait_for_timeout(100)
        path=shots/f'{stem}-{width}.png'
        await page.screenshot(path=str(path),animations='disabled')
        result['screenshots'][str(width)]=str(path.relative_to(ROOT))
        result['responsive'].append(await page.evaluate('({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,horizontalOverflow:document.documentElement.scrollWidth>innerWidth})'))
    await page.set_viewport_size(viewport)
    await page.goto(url,wait_until='networkidle')
    if page_name=='bookings.html':
        result['booking_dialog']=await booking_dialog(page)
        await page.goto(url,wait_until='networkidle')
        result['filters']=await booking_interactions(page,url)
    else:
        result['interactions']=await common.interaction_evidence(page)
    if stage==3:
        await page.goto(url,wait_until='networkidle')
        destination='bookings.html' if page_name=='index.html' else 'index.html'
        name='Bookings' if page_name=='index.html' else 'Overview'
        try:
            link=page.locator('aside, nav').get_by_role('link',name=re.compile('^'+name+'$',re.I)).first
            await link.click(timeout=1800)
            result['cross_page_navigation']={'works':page.url.split('?')[0].endswith('/'+destination),'destination':page.url}
        except Exception as error:
            result['cross_page_navigation']={'works':False,'error':str(error).splitlines()[0]}
    result.update(browser_errors=errors,external_requests=external,failed_requests=failed,
                  html_unchanged_by_evaluation=before==hashlib.sha256(html.read_bytes()).hexdigest())
    await context.close()
    return result

async def main():
    class Handler(SimpleHTTPRequestHandler):
        def log_message(self,*args):pass
    server=ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT)))
    threading.Thread(target=server.serve_forever,daemon=True).start()
    truth=json.loads((ROOT/'ground-truth.json').read_text())
    evaluator_hash=hashlib.sha256(Path(__file__).read_bytes()+(ROOT/'ground-truth.json').read_bytes()+(ROOT.parent/'web-reconstruction-2026-10-23/evaluate.py').read_bytes()).hexdigest()
    previous=json.loads((ROOT/'evaluation.json').read_text())if (ROOT/'evaluation.json').exists()else{}
    results=[]
    try:
        async with async_playwright() as playwright:
            browser=await playwright.chromium.launch(channel='chrome',headless=True)
            cases=json.loads((ROOT/'run-environment.json').read_text())['cases']
            for label in cases:
                for stage,name,key in [(1,'index.html','original'),(2,'index.html','revision'),(3,'index.html','revision'),(3,'bookings.html','bookings')]:
                    fingerprint=source_hash(ROOT/'outputs'/label/f'stage-{stage}')
                    cached=next((r for r in previous.get('results',[])if r['case']==label and r['stage']==stage and r['page']==name and r.get('status')=='rendered' and r.get('source_tree_sha256')==fingerprint and r.get('evaluator_sha256')==evaluator_hash),None)
                    result=cached or await render(browser,server.server_port,label,stage,name,truth[key])
                    result.update(source_tree_sha256=fingerprint,evaluator_sha256=evaluator_hash)
                    results.append(result)
                    save('evaluation.json',{'browser_version':browser.version,'results':results})
                    print(json.dumps({k:v for k,v in result.items() if k in ['case','stage','page','status','text_anchors_matched','pixel_metrics','marker_visible','superseded_text_visible','cross_page_navigation']}),flush=True)
            preservation=[]
            for label in cases:
                a=next((r for r in results if r['case']==label and r['stage']==2),{})
                b=next((r for r in results if r['case']==label and r['stage']==3 and r['page']=='index.html'),{})
                if a.get('screenshots') and b.get('screenshots'):
                    preservation.append({'case':label,'revision_retained':all(b['critical_values_visible'].values()) and b['marker_visible'] and not b['superseded_text_visible'],
                        'stage2_vs_stage3':metrics(ROOT/a['screenshots']['desktop'],ROOT/b['screenshots']['desktop']),
                        'stage2_vs_stage3_visible_text_equal':normalize(a['desktop_dom']['visibleText'])==normalize(b['desktop_dom']['visibleText'])})
            save('evaluation.json',{'browser_version':browser.version,'results':results,'dashboard_preservation':preservation})
            await browser.close()
    finally:
        server.shutdown();server.server_close()

if __name__=='__main__':asyncio.run(main())
