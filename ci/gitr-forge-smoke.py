#!/usr/bin/env python3
"""Actual forge CLI/HTTP + native redb actions + disposable compiled Rust Wasm."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import queue
import secrets
import subprocess
import tempfile
import threading
import urllib.error
import urllib.request

EXE = '.exe' if os.name == 'nt' else ''
BINARY = (Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / ('debug/gitr'+EXE)).resolve()
WASM = (Path(os.environ.get('GITR_EXAMPLE_TARGET_DIR', 'crates/action-example/target')) / 'wasm32-unknown-unknown/release/gitr_action_example.wasm').resolve()
TOKEN = secrets.token_hex(32)
ENV = {**os.environ,'GITR_API_TOKEN':TOKEN}

def git(repo,*args):
    p = subprocess.run(['git',*args],cwd=repo,capture_output=True,check=False,timeout=30)
    assert p.returncode == 0, p.stderr
    return p.stdout.decode().strip()

def native(repo,*args,body=None,ok=True):
    p = subprocess.run([str(BINARY),*args],cwd=repo,input=None if body is None else json.dumps(body).encode(),
        env={**ENV,'PATH':''},capture_output=True,check=False,timeout=40)
    if ok: assert p.returncode == 0,p.stderr
    return p

def control(repo,request,ok=True):
    result = native(repo,'forge','control',body=request,ok=ok)
    return json.loads(result.stdout) if ok else result

class Server:
    def __init__(self,repo):
        self.errors = tempfile.TemporaryFile(mode='w+')
        self.p = subprocess.Popen([str(BINARY),'actions','serve','--repository','qualification'],cwd=repo,
            env={**ENV,'PATH':''},stdout=subprocess.PIPE,stderr=self.errors,text=True)
        q = queue.Queue()
        def reader():
            parts = []
            for line in self.p.stdout:
                parts.append(line)
                try:
                    q.put(json.loads(''.join(parts))); return
                except json.JSONDecodeError:
                    if sum(map(len,parts)) > 4096: break
            q.put(None)
        threading.Thread(target=reader,daemon=True).start()
        try:
            ready = q.get(timeout=30)
            assert ready and ready['backend'] == 'redb'
            self.root = ready['endpoint'].removesuffix('/v1/actions')
        except Exception:
            self.close(); raise
    def close(self):
        if self.p.poll() is None: self.p.kill()
        self.p.wait(timeout=10)
        self.p.stdout.close(); self.errors.close()
    def call(self,request,path='forge',expected=200,token=TOKEN,scope='qualification',origin=None,raw=None):
        headers={'Content-Type':'application/json','x-gitr-repository':scope}
        if token is not None: headers['Authorization']='Bearer '+token
        if origin is not None: headers['Origin']=origin
        req=urllib.request.Request(self.root+'/v1/'+path,data=raw if raw is not None else json.dumps(request).encode(),headers=headers,method='POST')
        opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(req,timeout=40) as reply: status,data=reply.status,reply.read()
        except urllib.error.HTTPError as error: status,data=error.code,error.read()
        assert status in (expected if isinstance(expected,tuple) else (expected,)),(request.get('operation'),status,data[:400])
        assert TOKEN.encode() not in data
        return status,json.loads(data)

def export_check(data):
    lines=[json.loads(line) for line in data.splitlines()]
    assert lines[0]['format']=='gitr-forge-export-v1'
    digest=hashlib.sha256(); counts={'thread':0,'event':0,'receipt':0,'meta':0}
    for row in lines[1:-1]:
        key=bytes.fromhex(row['key_hex']); value=bytes.fromhex(row['data_hex'])
        digest.update(row['kind'].encode()+b'\0');digest.update(len(key).to_bytes(8,'big'));digest.update(key)
        digest.update(len(value).to_bytes(8,'big'));digest.update(value);counts[row['kind']]+=1
        if row['kind']!='meta': json.loads(value)
    footer=lines[-1]
    assert footer['sha256']==digest.hexdigest()
    assert footer['threads']==counts['thread'] and footer['events']==counts['event'] and footer['receipts']==counts['receipt']
    return counts

for fmt in ['sha1','sha256']:
    with tempfile.TemporaryDirectory(prefix='gitr-forge-') as temp:
        repo=Path(temp)/'repo';repo.mkdir()
        git(repo,'init','-q','--object-format='+fmt)
        git(repo,'config','user.name','Qualification');git(repo,'config','user.email','gitr@example.invalid')
        git(repo,'config','core.autocrlf','false')
        (repo/'source.rs').write_bytes(b'fn main() {}\n');git(repo,'add','source.rs');git(repo,'commit','-qm','base')
        base=git(repo,'rev-parse','HEAD')
        (repo/'source.rs').write_bytes(b'fn main() { println!("new"); }\n');git(repo,'add','source.rs');git(repo,'commit','-qm','proposal')
        head=git(repo,'rev-parse','HEAD')
        module=json.loads(native(repo,'actions','install',str(WASM)).stdout)['module_sha256']
        issue_request={'operation':'create_issue','operation_id':'issue-create','title':'  Track change  ','body':'line1\r\nline2'}
        issue=control(repo,issue_request);assert issue['number']==1 and issue['title']=='Track change' and issue['body']=='line1\nline2'
        pull_request={'operation':'create_pull','operation_id':'pull-create','title':'Source change','body':'Review this revision','base':'HEAD~1','head':'HEAD',
            'required_checks':{'source-policy':{'module_sha256':module,'path':'source.rs'}}}
        proposal=control(repo,pull_request)
        assert proposal['number']==2 and proposal['pull']['head_oid']==head and proposal['pull']['base_oid']==base
        native(repo,'actions','submit','old',module,'--revision',base,'--path','source.rs')
        native(repo,'actions','submit','current',module,'--revision',head,'--path','source.rs')
        server=Server(repo)
        try:
            get={'operation':'get','number':2}
            for token,status in [(None,401),('0'*64,401)]:server.call(get,expected=status,token=token)
            server.call(get,expected=403,scope='another-repository')
            server.call(get,expected=403,origin='https://untrusted.invalid')
            server.call({**get,'database':'/etc/passwd'},expected=400)
            server.call({**issue_request,'actor':'admin'},expected=400)
            server.call(get,expected=413,raw=b'x'*(2*1024*1024+16385))
            assert server.call(get)[1]==proposal
            attach={'operation':'attach_check','operation_id':'attach','number':2,'expected_revision':1,'context':'source-policy','job':'old'}
            server.call(attach,expected=422)
            attach['job']='current'
            linked=server.call(attach)[1];assert linked['revision']==2
            readiness=server.call({'operation':'readiness','number':2})[1]
            assert not readiness['locally_ready'] and readiness['waiting']==1
            server.call({**attach,'state':'success'},expected=400)
            (repo/'source.rs').write_bytes(b'dirty worktree is not checked  \n')
            result=server.call({'operation':'run','job':'current'},path='actions')[1]
            assert result['status']=='passed'
            review={'operation':'review','operation_id':'approve','number':2,'expected_revision':2,'head_oid':head,'decision':'approved'}
            reviewed=server.call(review)[1];assert reviewed['revision']==3
            readiness=json.loads(native(repo,'forge','check','2').stdout)
            assert readiness['locally_ready'] and readiness['passing']==1 and not readiness['authorizes_merge']
            assert readiness['head_oid']==head

            def edit(i):
                return server.call({'operation':'edit','operation_id':f'edit-{i}','number':2,'expected_revision':3,'title':f'Writer {i}','body':'body'},expected=(200,422))[0]
            with ThreadPoolExecutor(max_workers=4) as pool: results=list(pool.map(edit,range(4)))
            assert results.count(200)==1 and results.count(422)==3,results
            def comment(i):
                req={'operation':'comment','operation_id':f'comment-{i}','number':2,'body':f'Note {i}'}
                first=server.call(req)[1]
                assert server.call(req)[1]==first
                return first
            with ThreadPoolExecutor(max_workers=4) as pool:list(pool.map(comment,range(4)))
            state=server.call(get)[1];assert state['revision']==8 and state['comment_count']==4
            assert control(repo,pull_request)==proposal
            assert control(repo,issue_request)==issue
            page=server.call({'operation':'list','after':1,'limit':1})[1]
            assert page['items'][0]['number']==2 and page['next_after']==2
            events=server.call({'operation':'timeline','number':2,'after':0,'limit':8})[1]
            assert len(events['items'])==8 and events['next_after']==8

            # Move HEAD: the old operation key must return its exact original
            # receipt without resolving HEAD/HEAD~1 again or rewriting approval.
            git(repo,'add','source.rs');git(repo,'commit','-qm','new failed-policy revision')
            new_head=git(repo,'rev-parse','HEAD')
            assert control(repo,pull_request)==proposal
            retarget={'operation':'retarget','operation_id':'retarget','number':2,'expected_revision':8,'base':base,'head':'HEAD'}
            updated=server.call(retarget)[1]
            assert updated['revision']==9 and updated['pull']['head_oid']==new_head
            assert updated['pull']['review'] is None and updated['pull']['check_jobs']=={}
            server.call({**review,'operation_id':'stale-review','expected_revision':9},expected=422)
            server.call({**attach,'operation_id':'stale-check','expected_revision':9},expected=422)
            native(repo,'actions','submit','failed',module,'--revision',new_head,'--path','source.rs')
            failed=server.call({'operation':'run','job':'failed'},path='actions')[1];assert failed['status']=='failed'
            server.call({**attach,'operation_id':'failed-check','expected_revision':9,'job':'failed'})
            server.call({**review,'operation_id':'new-review','expected_revision':10,'head_oid':new_head})
            readiness=server.call({'operation':'readiness','number':2})[1]
            assert not readiness['locally_ready'] and readiness['failing']==1 and readiness['local_review_approved']
            artifact=server.call({'operation':'artifact','job':'current'},path='actions')[1]
            assert hashlib.sha256(bytes.fromhex(artifact['hex'])).hexdigest()==artifact['sha256']
            before=server.call(get)[1]
            server.close();server=Server(repo)
            assert server.call(get)[1]==before
            assert server.call({'operation':'artifact','job':'current'},path='actions')[1]==artifact
            assert server.call(pull_request)[1]==proposal
            linked=Path(temp)/'linked'
            git(repo,'worktree','add','--detach',str(linked),head)
            assert json.loads(native(linked,'forge','show','2').stdout)==before
            first=native(repo,'forge','export').stdout;counts=export_check(first)
            assert native(repo,'forge','export').stdout==first
            assert counts=={'thread':2,'event':12,'receipt':12,'meta':1},counts
            assert (repo/'.git/gitr/state.redb').is_file()
            assert not list((repo/'.git/gitr').glob('*.sqlite*'))
            git(repo,'fsck','--strict')
            print(json.dumps({'format':fmt,'forge_redb_cli_http_ephemeral_wasm':'passed','atomic_conflict_winners':1,
                'concurrent_comments':4,'stale_review_and_check':'rejected','retry_after_branch_move':'original receipt',
                'linked_worktree':'same forge','export_records':counts,'multiuser_auth':'not implemented','merge':'not performed'}),flush=True)
        finally:server.close()
print('GITR_FORGE_SMOKE=passed',flush=True)
