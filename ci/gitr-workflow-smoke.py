#!/usr/bin/env python3
"""Actual committed YAML -> Rbit matrix/DAG -> redb -> compiled Rust/Wasm.

Uses only disposable SHA-1/SHA-256 repositories. Gitr commands have an empty PATH;
reference Git configures and validates fixtures, never implements Gitr operations.
"""
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
import urllib.request

EXE = '.exe' if os.name == 'nt' else ''
GITR = (Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / ('debug/gitr' + EXE)).resolve()
OLD = Path('.gitr-ci/gitr-before' + EXE).resolve()
WASM = (Path(os.environ.get('GITR_EXAMPLE_TARGET_DIR', 'crates/action-example/target')) / 'wasm32-unknown-unknown/release/gitr_action_example.wasm').resolve()
TOKEN = secrets.token_hex(32)
ENV = {**os.environ, 'PATH':'', 'GITR_API_TOKEN':TOKEN}


def call(argv, cwd, native=False, ok=True):
    result = subprocess.run(argv, cwd=cwd, env=ENV if native else None, capture_output=True, timeout=40)
    if ok and result.returncode:
        raise AssertionError(result.stderr.decode(errors='replace')[:4000])
    return result


def workflow(module):
    def step(path):
        return f"      - uses: gitr/wasm@v1\n        with:\n          module: '{module}'\n          path: '{path}'\n"
    return ("name: Source checks\non: workflow_dispatch\njobs:\n  source:\n    runs-on: gitr-wasmi-v1\n"
        "    strategy:\n      max-parallel: 1\n      matrix:\n        file: [a.rs, b.rs]\n    steps:\n"
        + step('${{ matrix.file }}')
        + "  final:\n    needs: source\n    runs-on: gitr-wasmi-v1\n    steps:\n" + step('a.rs')).encode()


class Api:
    def __init__(self, root):
        self.errors = tempfile.TemporaryFile()
        self.process = subprocess.Popen([str(GITR), 'actions', 'serve', '--repository', 'workflow-test'], cwd=root,
            env=ENV, stdout=subprocess.PIPE, stderr=self.errors, text=True)
        ready = queue.Queue()
        def read():
            data = ''
            while line := self.process.stdout.readline():
                data += line
                if len(data) > 4096: break
                try:
                    ready.put(json.loads(data))
                    return
                except json.JSONDecodeError: pass
            ready.put(None)
        threading.Thread(target=read, daemon=True).start()
        try:
            result = ready.get(timeout=20)
            assert result and result['backend'] == 'redb'
            self.endpoint = result['endpoint']
        except Exception:
            self.close()
            raise
    def run(self, job):
        request = urllib.request.Request(self.endpoint, data=json.dumps({'operation':'run','job':job}).encode(),
            headers={'Content-Type':'application/json', 'Authorization':'Bearer '+TOKEN,'x-gitr-repository':'workflow-test'})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=20) as response:
            return json.load(response)
    def close(self):
        if self.process.poll() is None: self.process.kill()
        self.process.wait(timeout=10)
        self.process.stdout.close()
        self.errors.close()


def verify_export(raw):
    rows = [json.loads(line) for line in raw.splitlines()]
    assert rows[0]['format'] == 'gitr-action-export-v2'
    digest = hashlib.sha256()
    counts = {'object':0,'job':0,'workflow':0,'workflow_owner':0}
    for row in rows[1:-1]:
        kind = row['kind']
        counts[kind] += 1
        if kind == 'object':
            key, data = row['sha256'], bytes.fromhex(row['data_hex'])
            assert hashlib.sha256(data).hexdigest() == key
        elif kind in ('job','workflow'):
            key, data = row['id'], bytes.fromhex(row['json_hex'])
            assert json.loads(data) == row['record']
        else:
            key, data = row['id'], row['run_id'].encode()
        key = key.encode()
        digest.update(kind.encode() + b'\0')
        digest.update(len(key).to_bytes(8,'big')); digest.update(key)
        digest.update(len(data).to_bytes(8,'big')); digest.update(data)
    assert rows[-1]['sha256'] == digest.hexdigest()
    assert rows[-1]['workflows'] == counts['workflow']
    assert counts['workflow_owner'] == sum(len(row['record']['plan']['tasks']) for row in rows if row['kind'] == 'workflow')
    return counts


for fmt in ['sha1', 'sha256']:
    with tempfile.TemporaryDirectory(prefix='gitr-workflows-') as directory:
        root = Path(directory)
        def git(*args): return call(['git', *args], root)
        def native(*args, ok=True): return call([str(GITR), 'actions', *args], root, True, ok)
        def command(*args, ok=True): return native('workflow', *args, ok=ok)
        git('init','-q','--object-format='+fmt,'-b','main')
        git('config','user.name','Workflow qualification')
        git('config','user.email','gitr@example.invalid')
        git('config','core.autocrlf','false')
        sources = {'a.rs':b'fn main() {}\n', 'b.rs':b'pub fn value() -> u8 { 7 }\n'}
        for name, data in sources.items(): (root/name).write_bytes(data)
        module = json.loads(native('install',str(WASM)).stdout)['module_sha256']
        (root/'workflow.yml').write_bytes(workflow(module))
        git('add','a.rs','b.rs','workflow.yml'); git('commit','-qm','immutable workflow and inputs')
        commit = git('rev-parse','HEAD').stdout.decode().strip()
        state = root/'.git/gitr/state.redb'
        before = hashlib.sha256(state.read_bytes()).digest()
        plan = json.loads(command('plan','checks','--path','workflow.yml').stdout)
        assert hashlib.sha256(state.read_bytes()).digest() == before, 'dry planning wrote the database'
        assert len(plan['tasks']) == 3 and plan['commit_oid'] == commit
        submitted = json.loads(command('submit','checks','--path','workflow.yml').stdout)
        assert submitted['plan'] == plan and submitted['status'] == 'pending'
        final_job = plan['tasks'][2]['action_id']
        blocked = native('run',final_job,ok=False)
        assert blocked.returncode and json.loads(blocked.stdout)['attempt'] == 0
        if OLD.exists():
            refused = call([str(OLD),'actions','run',final_job],root,True,False)
            assert refused.returncode and not refused.stdout, 'old worker accepted managed job'
            refused = call([str(OLD),'actions','export'],root,True,False)
            assert refused.returncode and not refused.stdout, 'old export lost workflow ownership'
        api = Api(root)
        try:
            assert api.run(final_job)['status'] == 'queued', 'API bypassed dependencies'
            # Both the workflow and source on disk are now hostile/dirty. Neither
            # is consulted by an already published immutable plan.
            (root/'a.rs').write_bytes(b'dirty trailing spaces  \n')
            (root/'workflow.yml').write_bytes(b'run: never execute this worktree content\n')
            with ThreadPoolExecutor(max_workers=4) as pool:
                results = list(pool.map(lambda _: command('run','checks',ok=False), range(4)))
            assert all(result.stdout for result in results), [r.stderr for r in results]
            result = json.loads(command('run','checks').stdout)
            assert result['status'] == 'passed' and all(job['attempt'] == 1 for job in result['jobs'])
            for task in result['plan']['tasks']:
                report = json.loads(native('artifact',task['action_id']).stdout)
                content = sources[task['spec']['path']]
                assert task['spec']['input_sha256'] == hashlib.sha256(content).hexdigest()
                assert report == {'bytes':len(content),'lines':1,'trailing_whitespace':False}
            assert json.loads(command('submit','checks','--revision',commit,'--path','workflow.yml').stdout) == result
        finally:
            api.close()
        # Every CLI request is already a separate process. Reopening keeps results.
        assert json.loads(command('status','checks').stdout) == result
        assert json.loads(command('cancel','checks').stdout) == result
        (root/'a.rs').write_bytes(sources['a.rs'])
        (root/'workflow.yml').write_bytes(workflow(module))
        (root/'b.rs').write_bytes(b'bad input with trailing spaces  \n')
        git('add','.'); git('commit','-qm','policy violation')
        assert command('submit','checks','--path','workflow.yml',ok=False).returncode
        command('submit','bad','--path','workflow.yml')
        failed = command('run','bad',ok=False)
        failure = json.loads(failed.stdout)
        assert failed.returncode and failure['status'] == 'failed'
        assert failure['jobs'][2]['status'] == 'cancelled' and failure['jobs'][2]['attempt'] == 0
        assert native('artifact',failure['jobs'][2]['id'],ok=False).returncode
        command('submit','cancel','--revision',commit,'--path','workflow.yml')
        cancelled = json.loads(command('cancel','cancel').stdout)
        assert cancelled['status'] == 'cancelled' and all(job['attempt'] == 0 for job in cancelled['jobs'])
        assert json.loads(command('run','cancel',ok=False).stdout) == cancelled
        export = native('export').stdout
        counts = verify_export(export)
        assert counts['workflow'] == 3 and native('export').stdout == export
        git('fsck','--strict')
        print(json.dumps({'format':fmt,'workflow_rbit_redb_rust_wasm':'passed','matrix_tasks':3,
            'dependency_gates':'CLI and HTTP','concurrent_processes':4,'old_worker_refusal':OLD.exists(),
            'dirty_worktree':'excluded','failure_and_cancel':'fenced','export_counts':counts}),flush=True)
print('GITR_WORKFLOW_SMOKE=passed',flush=True)
