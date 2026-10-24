#!/usr/bin/env python3
"""Hash persisted image inputs without copying session text or credentials."""
import base64
import hashlib
from io import BytesIO
import json
from pathlib import Path
from PIL import Image

ROOT=Path(__file__).resolve().parent

def main():
    env=json.loads((ROOT/'run-environment.json').read_text())
    refs={r['sha256']:name for name,r in env['image_metadata'].items()}
    def describe(uri):
        if not uri.startswith('data:'):return {'encoding':'other'}
        raw=base64.b64decode(uri.split(',',1)[1]);sha=hashlib.sha256(raw).hexdigest()
        image=Image.open(BytesIO(raw))
        return {'sha256':sha,'matches_reference':refs.get(sha),'dimensions':list(image.size),
                'format':image.format,'encoding':'data_uri','encoded_bytes':len(raw)}
    records=[]
    for label in env['cases']:
        first=json.loads((ROOT/f'{label}-stage-1-run.json').read_text())
        sid=first.get('session_id')
        images=[]
        if label.startswith('smith'):
            storage=str(Path(env['scratch'])/'sessions')
            for stage in [1,2,3]:
                path=ROOT/f'{label}-stage-{stage}-run.json'
                if path.exists():storage=json.loads(path.read_text()).get('session_storage',storage)
            files=list(Path(storage).rglob(str(sid)+'.snapshot.json'))
            if not files:continue
            snapshot=json.loads(files[0].read_text())['snapshot']
            for i,message in enumerate(snapshot['history']):
                for j,part in enumerate(message.get('content',[])):
                    if part.get('type')=='image':images.append({'message':i,'part':j,**describe(part.get('url',''))})
        else:
            files=list((Path.home()/'.codex/sessions/2026/10/23').glob('*'+str(sid)+'.jsonl'))
            if not files:continue
            for line_number,line in enumerate(files[0].open(),1):
                row=json.loads(line)
                item=row.get('payload',{})
                if row.get('type')=='response_item' and item.get('role')=='user':
                    for j,part in enumerate(item.get('content',[])):
                        if part.get('type')=='input_image':images.append({'line':line_number,'part':j,**describe(part.get('image_url',''))})
        expected=['part-1.png','part-2.png','part-3.png','revision.png','bookings.png','revision.png']
        records.append({'case':label,'session_id':sid,'images':images,
            'all_six_images_retained_in_order':[r.get('matches_reference') for r in images]==expected,
            'all_persisted_image_bytes_match_source_files':all(r.get('matches_reference') for r in images),
            'scope':'Persisted canonical inputs; not an interception of every outgoing native request.'})
    (ROOT/'image-history-audit.json').write_text(json.dumps(records,indent=2)+'\n')
    print(json.dumps([{k:v for k,v in r.items() if k!='images'} for r in records],indent=2))

if __name__=='__main__':main()
