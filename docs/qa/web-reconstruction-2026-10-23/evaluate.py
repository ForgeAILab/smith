#!/usr/bin/env python3
"""Render frozen benchmark outputs and collect visual, DOM and interaction evidence."""

import argparse
import asyncio
import functools
import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import threading

import numpy as np
from PIL import Image, ImageFilter
from playwright.async_api import async_playwright
from skimage.metrics import structural_similarity

ROOT = Path(__file__).resolve().parent
TRUTH = json.loads((ROOT / "ground-truth.json").read_text())


def normalize(text):
    return " ".join(re.sub(r"[^a-z0-9]+", " ", text.lower()).split())


def contains(text, needle):
    return (" " + normalize(needle) + " ") in (" " + normalize(text) + " ")


def overlap(a, b):
    x, y = max(a[0], b[0]), max(a[1], b[1])
    right, bottom = min(a[0] + a[2], b[0] + b[2]), min(a[1] + a[3], b[1] + b[3])
    intersection = max(0, right - x) * max(0, bottom - y)
    return intersection / max(1, a[2] * a[3] + b[2] * b[3] - intersection)


def image_metrics(path):
    reference = Image.open(ROOT / "reference.png").convert("RGB")
    actual = Image.open(path).convert("RGB")
    a, b = np.asarray(reference), np.asarray(actual)
    blurred_a = np.asarray(reference.filter(ImageFilter.GaussianBlur(3)))
    blurred_b = np.asarray(actual.filter(ImageFilter.GaussianBlur(3)))
    return {"ssim": round(float(structural_similarity(a, b, channel_axis=2, data_range=255)), 5),
            "blurred_ssim_radius_3": round(float(structural_similarity(blurred_a, blurred_b, channel_axis=2, data_range=255)), 5),
            "mean_absolute_rgb_error": round(float(np.abs(a.astype(float) - b.astype(float)).mean()), 4)}


async def dom_evidence(page):
    return await page.evaluate("""() => {
      const clean = s => (s || '').replace(/\\s+/g, ' ').trim();
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      const visibleText = []; let node;
      while ((node = walker.nextNode())) {
        const parent = node.parentElement;
        if (!parent || parent.closest('script, style, noscript, template')) continue;
        const box = parent.getBoundingClientRect(), style = getComputedStyle(parent);
        if (box.width && box.height && style.visibility !== 'hidden' && style.display !== 'none') visibleText.push(node.textContent);
      }
      document.querySelectorAll('select,input:not([type="hidden"])').forEach(e => {
        const box=e.getBoundingClientRect(),style=getComputedStyle(e);
        if(box.width && box.height && style.visibility!=='hidden' && style.display!=='none')
          visibleText.push(e.tagName==='SELECT' ? [...e.selectedOptions].map(o=>o.textContent).join(' ') : e.value);
      });
      return {
        text: document.body.innerText,
        visibleText: clean(visibleText.join(' ')),
        title: document.title,
        scrollWidth: document.documentElement.scrollWidth,
        viewportWidth: innerWidth,
        tableRows: document.querySelectorAll('table tbody tr').length,
        buttons: [...document.querySelectorAll('button')].map(e => ({text: clean(e.innerText || e.getAttribute('aria-label')), disabled:e.disabled})),
        referenceAssetUsed: [...document.querySelectorAll('img, image')].some(e => /reference\\.png/i.test(e.getAttribute('src') || e.getAttribute('href') || '')) || /url\\([^)]*reference\\.png/i.test(document.documentElement.outerHTML),
        elements: [...document.querySelectorAll('body *')].filter(e => clean(e.innerText || e.textContent).length < 90).map(e => {
          const r = e.getBoundingClientRect();
          const boxes = []; let parent = e;
          for (let i = 0; parent && i < 8; i++, parent = parent.parentElement) {
            const b = parent.getBoundingClientRect();
            boxes.push([b.x, b.y, b.width, b.height]);
          }
          return {tag:e.tagName, text:clean(e.innerText || e.textContent), bbox:[r.x,r.y,r.width,r.height], ancestors:boxes};
        }).filter(e => e.bbox[2] && e.bbox[3])
      };
    }""")


async def interaction_evidence(page):
    result = {"new_booking_dialog_opens": False, "escape_closes_dialog": False,
              "booking_submission_adds_row": False, "chart_period_changes_state": False,
              "sidebar_selection_changes": False, "notes": []}
    button = page.get_by_role("button", name=re.compile(r"new booking", re.I))
    if not await button.count():
        button = page.get_by_text(re.compile(r"new booking", re.I))
    try:
        await button.first.click(timeout=1500)
        await page.wait_for_timeout(150)
        dialog = page.locator('dialog[open], [role="dialog"]:visible').first
        if await dialog.count() and await dialog.is_visible():
            result["new_booking_dialog_opens"] = True
            await page.keyboard.press("Escape")
            await page.wait_for_timeout(100)
            result["escape_closes_dialog"] = not await dialog.is_visible()
            await button.first.click(timeout=1500)
            dialog = page.locator('dialog[open], [role="dialog"]:visible').first
            inputs = dialog.locator('input:not([type="hidden"]), textarea, select')
            for index in range(await inputs.count()):
                field = inputs.nth(index)
                tag, kind = await field.evaluate('(e) => [e.tagName, e.type]')
                if tag == "SELECT":
                    options = await field.locator('option').evaluate_all('(a) => a.filter(e=>e.value && !e.disabled).map(e=>e.value)')
                    if options:
                        await field.select_option(options[0])
                elif kind in ["text", "search", "textarea", "tel"]:
                    await field.fill("Benchmark Guest")
                elif kind == "email":
                    await field.fill("benchmark@example.test")
                elif kind == "date":
                    await field.fill("2026-10-24")
                elif kind == "time":
                    await field.fill("10:00")
                elif kind == "datetime-local":
                    await field.fill("2026-10-24T10:00")
                elif kind == "number":
                    await field.fill("1")
            before = await page.locator('table tbody tr').count()
            submit = dialog.locator('button[type="submit"], input[type="submit"]')
            if not await submit.count():
                submit = dialog.get_by_role('button', name=re.compile(r'create|save|add|confirm|book', re.I))
            await submit.last.click(timeout=1500)
            await page.wait_for_timeout(150)
            result["booking_submission_adds_row"] = await page.locator('table tbody tr').count() > before
    except Exception as error:
        result["notes"].append("Dialog/form smoke check: " + str(error).splitlines()[0])
    await page.reload()
    await page.wait_for_timeout(250)
    try:
        month = page.get_by_role('button', name=re.compile(r'^month$', re.I)).first
        before = await page.locator('svg').evaluate_all('(a) => a.map(e=>e.outerHTML).join("")')
        class_before = await month.get_attribute('class')
        await month.click(timeout=1500)
        after = await page.locator('svg').evaluate_all('(a) => a.map(e=>e.outerHTML).join("")')
        result['chart_period_changes_state'] = (before != after or class_before != await month.get_attribute('class')
                                                or await month.get_attribute('aria-selected') == 'true'
                                                or await month.get_attribute('aria-pressed') == 'true')
    except Exception as error:
        result['notes'].append('Chart smoke check: ' + str(error).splitlines()[0])
    try:
        nav = page.locator('aside, nav').get_by_text(re.compile(r'^bookings$', re.I)).first
        before = await nav.evaluate('(e) => e.outerHTML + e.parentElement.outerHTML')
        await nav.click(timeout=1500)
        result['sidebar_selection_changes'] = before != await nav.evaluate('(e) => e.outerHTML + e.parentElement.outerHTML')
    except Exception as error:
        result['notes'].append('Navigation smoke check: ' + str(error).splitlines()[0])
    return result


async def evaluate_case(browser, label, port):
    folder = ROOT / 'outputs' / label
    screenshot_folder = ROOT / 'screenshots'
    screenshot_folder.mkdir(exist_ok=True)
    html = folder / 'index.html'
    if not html.exists():
        return {'case': label, 'status': 'no_index_html'}
    before_hash = hashlib.sha256(html.read_bytes()).hexdigest()
    context = await browser.new_context(viewport=TRUTH['viewport'], device_scale_factor=1, reduced_motion='reduce')
    page = await context.new_page()
    errors, external_requests = [], []
    page.on('pageerror', lambda error: errors.append(str(error)))
    async def route(request):
        if request.request.url.startswith(f'http://127.0.0.1:{port}/') or request.request.url.startswith('data:'):
            await request.continue_()
        else:
            external_requests.append(request.request.url)
            await request.abort()
    await page.route('**/*', route)
    await page.goto(f'http://127.0.0.1:{port}/outputs/{label}/index.html', wait_until='networkidle')
    await page.evaluate('document.fonts.ready')
    await page.wait_for_timeout(500)
    desktop = screenshot_folder / (label + '-desktop.png')
    await page.screenshot(path=str(desktop), animations='disabled')
    dom = await dom_evidence(page)
    matched = [text for text in TRUTH['text_anchors'] if contains(dom['visibleText'], text)]
    missing = [text for text in TRUTH['text_anchors'] if text not in matched]
    regions = []
    for region in TRUTH['regions']:
        boxes = [box for element in dom['elements'] if normalize(element['text']) == normalize(region['anchor']) for box in element['ancestors']]
        best = max(boxes, key=lambda box: overlap(box, region['bbox'])) if boxes else None
        regions.append({**region, 'actual_bbox': best, 'iou': round(overlap(best, region['bbox']), 4) if best else 0})
    observations = None
    try:
        observations = json.loads((folder / 'observations.json').read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    result = {'case':label, 'status':'rendered', 'html_sha256':before_hash,
              'text_anchors_matched':len(matched), 'text_anchors_total':len(TRUTH['text_anchors']),
              'text_coverage_percent':round(100*len(matched)/len(TRUTH['text_anchors']),2),
              'missing_text':missing, 'recognition_marker_correct':contains(dom['visibleText'],TRUTH['recognition_marker']),
              'observations':observations, 'major_regions':regions,
              'mean_region_iou':round(sum(r['iou'] for r in regions)/len(regions),4),
              'desktop_dom':{k:v for k,v in dom.items() if k != 'elements'},
              'pixel_metrics':image_metrics(desktop), 'responsive':[]}
    for width in [768,375]:
        await page.set_viewport_size({'width':width,'height':1024})
        await page.wait_for_timeout(200)
        await page.screenshot(path=str(screenshot_folder/(label+f'-{width}.png')),animations='disabled')
        result['responsive'].append(await page.evaluate('({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,horizontalOverflow:document.documentElement.scrollWidth>innerWidth})'))
    await page.set_viewport_size(TRUTH['viewport'])
    await page.reload()
    await page.wait_for_timeout(250)
    result['interactions'] = await interaction_evidence(page)
    result['browser_errors'] = errors
    result['external_requests'] = external_requests
    result['html_unchanged_by_evaluation'] = hashlib.sha256(html.read_bytes()).hexdigest() == before_hash
    await context.close()
    return result


async def main(labels):
    class Handler(SimpleHTTPRequestHandler):
        def log_message(self,*args):
            pass
    server = ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT)))
    threading.Thread(target=server.serve_forever,daemon=True).start()
    existing = json.loads((ROOT/'evaluation.json').read_text()) if (ROOT/'evaluation.json').exists() else {}
    results = [result for result in existing.get('results', []) if result['case'] not in labels]
    try:
        async with async_playwright() as playwright:
            browser=await playwright.chromium.launch(channel='chrome',headless=True)
            for label in labels:
                result=await evaluate_case(browser,label,server.server_port)
                results.append(result)
                (ROOT/'evaluation.json').write_text(json.dumps({'browser_version':browser.version,'results':results},indent=2,ensure_ascii=False)+'\n')
                print(json.dumps({k:v for k,v in result.items() if k in ['case','status','text_coverage_percent','mean_region_iou','pixel_metrics','interactions']}),flush=True)
            await browser.close()
    finally:
        server.shutdown();server.server_close()


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case',action='append',choices=['smith-sol','codex-sol','smith-astra','codex-astra'])
    args=parser.parse_args()
    asyncio.run(main(args.case or ['smith-sol','codex-sol','smith-astra','codex-astra']))
