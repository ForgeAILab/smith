#!/usr/bin/env python3
"""Smoke-test the offline comparison viewer, including all checkpoint/arm/viewport combinations."""
import asyncio
import functools
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
from playwright.async_api import async_playwright

ROOT=Path(__file__).resolve().parent

async def main():
    class Handler(SimpleHTTPRequestHandler):
        def log_message(self,*args):pass
    server=ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT.parent)))
    threading.Thread(target=server.serve_forever,daemon=True).start()
    errors=[];checks=[]
    try:
        async with async_playwright() as playwright:
            browser=await playwright.chromium.launch(channel='chrome',headless=True)
            page=await browser.new_page(viewport={'width':1440,'height':1000},device_scale_factor=1)
            page.on('pageerror',lambda e:errors.append(str(e)))
            url=f'http://127.0.0.1:{server.server_port}/{ROOT.name}/comparison.html'
            await page.goto(url,wait_until='networkidle')
            for checkpoint in ['0','1','2','3']:
                await page.select_option('#checkpoint',checkpoint)
                for arm in ['smith-sol','smith-astra','codex-sol','codex-astra']:
                    await page.select_option('#arm',arm)
                    for width in ['desktop','768','375']:
                        await page.select_option('#viewport',width)
                        await page.wait_for_function('Array.from(document.querySelectorAll("#viewer img:not([hidden])")).every(i=>i.complete&&i.naturalWidth>0)')
                        checks.append({'checkpoint':checkpoint,'arm':arm,'viewport':width,'images_loaded':True})
            await page.select_option('#viewport','desktop')
            await page.get_by_role('button',name='Overlay',exact=True).click()
            await page.locator('#opacity').evaluate('(e)=>{e.value="80";e.dispatchEvent(new Event("input",{bubbles:true}));}')
            opacity=await page.locator('#actual').evaluate('(e)=>getComputedStyle(e).opacity')
            assert opacity=='0.8',opacity
            await page.get_by_role('button',name='Reference',exact=True).click()
            assert await page.locator('#actual').evaluate('(e)=>getComputedStyle(e).opacity')=='0'
            await page.get_by_role('button',name='Result',exact=True).click()
            await page.locator('#opacity').focus()
            await page.keyboard.press('ArrowRight')
            assert await page.locator('#viewer').evaluate('(e)=>e.style.getPropertyValue("--opacity")')=='0.81'
            for summary in await page.locator('summary').all():
                await summary.click()
                assert await summary.evaluate('(e)=>e.parentElement.open')
                await summary.click()
            await page.locator('details').evaluate_all('(a)=>a.forEach(e=>e.open=true)')
            links=await page.locator('a[href]:visible').evaluate_all('(a)=>a.map(e=>e.href)')
            statuses=[]
            for link in links:
                response=await page.request.get(link)
                statuses.append({'url':link,'status':response.status})
                assert response.ok,link
            await page.locator('details').evaluate_all('(a)=>a.forEach(e=>e.open=false)')
            await page.select_option('#checkpoint','2')
            await page.select_option('#arm','codex-astra')
            await page.screenshot(path=str(ROOT/'viewer-desktop.png'),full_page=True)
            responsive=[]
            for width in [768,375]:
                await page.set_viewport_size({'width':width,'height':1000})
                await page.evaluate('window.scrollTo(0,0)')
                await page.wait_for_timeout(250)
                overflow=await page.evaluate('document.documentElement.scrollWidth>innerWidth')
                responsive.append({'width':width,'horizontal_overflow':overflow})
                assert not overflow,width
            await page.screenshot(path=str(ROOT/'viewer-mobile.png'))
            await page.locator('#viewer').scroll_into_view_if_needed()
            await page.wait_for_timeout(150)
            await page.screenshot(path=str(ROOT/'viewer-mobile-comparison.png'))
            result={'browser_version':browser.version,'selection_checks':checks,'selection_count':len(checks),
                    'overlay_opacity':opacity,'reference_result_modes':True,'keyboard_slider':True,
                    'disclosures':True,'visible_link_checks':statuses,'responsive':responsive,'javascript_errors':errors}
            assert not errors,errors
            (ROOT/'viewer-verification.json').write_text(json.dumps(result,indent=2)+'\n')
            await browser.close()
            print(json.dumps({k:v for k,v in result.items() if k not in ['selection_checks','visible_link_checks']}))
    finally:
        server.shutdown();server.server_close()

if __name__=='__main__':asyncio.run(main())
