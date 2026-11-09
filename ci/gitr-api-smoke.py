#!/usr/bin/env python3
"""Real optional API, native CLI and Rust/Wasm share the existing redb database."""
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
import time
import urllib.error
import urllib.request

EXE = '.exe' if os.name == 'nt' else ''
GITR = Path('rust/gitr/target/debug/gitr' + EXE).resolve()
WASM = Path('rust/gitr-action-example/target/wasm32-unknown-unknown/release/gitr_action_example.wasm').resolve()
TOKEN = secrets.token_hex(32)
ENV = {**os.environ, 'GITR_API_TOKEN': TOKEN}
SCOPE = 'qualification'


def run(argv, cwd, ok=True):
    result = subprocess.run(argv, cwd=cwd, env=ENV, capture_output=True, text=True, timeout=40)
    if ok and result.returncode:
        raise AssertionError(result.stderr[:3000])
    return result


class Server:
    def __init__(self, repo):
        self.errors = tempfile.TemporaryFile(mode='w+')
        self.process = subprocess.Popen([str(GITR), 'actions', 'serve', '--repository', SCOPE],
            cwd=repo, env=ENV, stdout=subprocess.PIPE, stderr=self.errors, text=True)
        lines = queue.Queue()
        # The CLI prints a small pretty-printed JSON readiness object.
        def read_ready():
            parts = []
            while line := self.process.stdout.readline():
                parts.append(line)
                try:
                    lines.put(json.loads(''.join(parts)))
                    return
                except json.JSONDecodeError:
                    if sum(map(len, parts)) > 4096:
                        break
            lines.put(None)
        threading.Thread(target=read_ready, daemon=True).start()
        try:
            ready = lines.get(timeout=30)
            assert ready and ready['backend'] == 'redb' and ready['repository'] == SCOPE
            self.endpoint = ready['endpoint']
        except Exception:
            self.stop()
            self.errors.seek(0)
            raise AssertionError('API startup failed: ' + self.errors.read()[:3000])

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=10)
        self.process.stdout.close()

    def close(self):
        self.stop()
        self.errors.close()

    def call(self, value, expected=200, token=TOKEN, scope=SCOPE, origin=None, raw=None, media='application/json'):
        headers = {'Content-Type': media, 'x-gitr-repository': scope}
        if token is not None:
            headers['Authorization'] = 'Bearer ' + token
        if origin is not None:
            headers['Origin'] = origin
        request = urllib.request.Request(self.endpoint, data=raw if raw is not None else json.dumps(value).encode(), headers=headers, method='POST')
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(request, timeout=30) as response:
                status, data = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, data = error.code, error.read()
        assert status == expected, (value.get('operation'), status, expected, data[:300])
        assert TOKEN.encode() not in data
        return json.loads(data)


def phase(name):
    print('PASS ' + name, flush=True)


# Small canonical core-Wasm fixture: memory export plus run() -> i32 that loops.
# It is installed through the real validating runtime before any crash test.
def leb(value):
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def section(kind, body):
    return bytes([kind]) + leb(len(body)) + body


def name(value):
    value = value.encode()
    return leb(len(value)) + value


loop_body = bytes.fromhex('00 03 40 0c 00 0b 41 00 0b')
LOOP = (b'\0asm\x01\0\0\0' + section(1, bytes.fromhex('01 60 00 01 7f'))
    + section(3, bytes.fromhex('01 00')) + section(5, bytes.fromhex('01 00 01'))
    + section(7, b'\x02' + name('memory') + b'\x02\0' + name('run') + b'\0\0')
    + section(10, b'\x01' + leb(len(loop_body)) + loop_body))

with tempfile.TemporaryDirectory(prefix='gitr-redb-api-') as directory:
    repo = Path(directory)
    run(['git', 'init', '-q'], repo)
    run(['git', 'config', 'user.name', 'Gitr qualification'], repo)
    run(['git', 'config', 'user.email', 'gitr@example.invalid'], repo)
    run(['git', 'config', 'core.autocrlf', 'false'], repo)
    (repo / 'source.rs').write_bytes(b'fn main() {}\n')
    run(['git', 'add', 'source.rs'], repo)
    run(['git', 'commit', '-qm', 'committed source'], repo)
    def native(*args, ok=True):
        return run([str(GITR), 'actions', *args], repo, ok)
    module = json.loads(native('install', str(WASM)).stdout)['module_sha256']
    native('submit', 'existing', module, '--path', 'source.rs')
    assert (repo / '.git/gitr/state.redb').exists()
    server = Server(repo)
    try:
        status = {'operation':'status', 'job':'existing'}
        server.call(status, 401, token=None)
        server.call(status, 401, token='0' * 64)
        server.call(status, 403, scope='other')
        server.call(status, 403, origin='https://untrusted.invalid')
        server.call(status, 415, media='text/plain')
        server.call({**status, 'database':'/etc/passwd'}, 400)
        server.call({'operation':'finish', 'job':'existing', 'attempt':1}, 400)
        server.call(status, 413, raw=b'x' * (2 * 1024 * 1024 + 16385))
        server.call({'operation':'install', 'wasm_hex':'00'}, 422)
        assert server.call(status)['status'] == 'queued'
        phase('authentication, scope, denied browser origin, typed/body limits and existing redb visibility')

        assert server.call({'operation':'install','wasm_hex':WASM.read_bytes().hex()})['module_sha256'] == module
        (repo / 'source.rs').write_bytes(b'dirty bytes with trailing spaces  \n')
        def submit(job, module=module):
            return server.call({'operation':'submit','job':job,'module':module,'revision':'HEAD','path':'source.rs'})
        submitted = submit('policy')
        assert submitted['spec']['input_sha256'] == hashlib.sha256(b'fn main() {}\n').hexdigest()
        passed = server.call({'operation':'run','job':'policy'})
        assert passed['status'] == 'passed' and passed['attempt'] == 1
        assert json.loads(native('status', 'policy').stdout) == passed
        artifact = server.call({'operation':'artifact','job':'policy'})
        output = bytes.fromhex(artifact['hex'])
        assert hashlib.sha256(output).hexdigest() == artifact['sha256']
        assert json.loads(output) == {'bytes':13,'lines':1,'trailing_whitespace':False}
        assert json.loads(native('artifact','policy').stdout) == json.loads(output)
        assert server.call({'operation':'run','job':'policy'}) == passed
        assert submit('policy') == passed
        assert json.loads(native('run','existing').stdout)['status'] == 'passed'
        assert server.call({'operation':'status','job':'existing'})['status'] == 'passed'
        phase('actual Rust/Wasm execution, immutable Git input and bidirectional native CLI/API interoperability')

        with ThreadPoolExecutor(max_workers=4) as pool:
            copies = list(pool.map(lambda _: submit('race'), range(4)))
        assert all(row == copies[0] for row in copies)
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda _: server.call({'operation':'run','job':'race'}), range(4)))
        final = server.call({'operation':'status','job':'race'})
        assert final['status'] == 'passed' and final['attempt'] == 1
        phase('concurrent API submissions/runs publish one attempt and preserve idempotency')

        submit('cancelled')
        assert server.call({'operation':'cancel','job':'cancelled'})['status'] == 'cancelled'
        assert server.call({'operation':'run','job':'cancelled'})['status'] == 'cancelled'
        server.call({'operation':'artifact','job':'cancelled'}, 422)
        assert server.call({'operation':'cancel','job':'policy'}) == passed
        run(['git','add','source.rs'], repo)
        run(['git','commit','-qm','policy violation'], repo)
        server.call({'operation':'submit','job':'policy','module':module,'revision':'HEAD','path':'source.rs'}, 422)
        submit('failed')
        failed = server.call({'operation':'run','job':'failed'})
        assert failed['status'] == 'failed' and failed['exit_code'] == 1
        server.call({'operation':'artifact','job':'failed'}, 422)
        phase('cancellation, changed-input conflicts and real failing guest publish no success artifact')

        loop_module = server.call({'operation':'install','wasm_hex':LOOP.hex()})['module_sha256']
        submit('interrupted', loop_module)
        with tempfile.TemporaryFile(mode='w+') as worker_output:
            worker = subprocess.Popen([str(GITR),'actions','run','interrupted'], cwd=repo, env=ENV,
                stdout=worker_output, stderr=worker_output, text=True)
            try:
                deadline = time.monotonic() + 10
                while True:
                    claimed = server.call({'operation':'status','job':'interrupted'})
                    if claimed['status'] == 'running':
                        break
                    if worker.poll() is not None or time.monotonic() > deadline:
                        raise AssertionError('worker did not expose a durable claim')
                    time.sleep(0.005)
                worker.kill()
                worker.wait(timeout=10)
            finally:
                if worker.poll() is None:
                    worker.kill()
                    worker.wait(timeout=10)
        phase('native worker can be killed after its durable claim while API remains responsive')
        server.close()
        server = Server(repo)
        assert server.call(status)['status'] == 'passed'
        assert server.call({'operation':'status','job':'policy'}) == passed
        assert server.call({'operation':'artifact','job':'policy'}) == artifact
        assert server.call({'operation':'status','job':'interrupted'})['status'] == 'running'
        phase('forced API restart preserves existing CLI and API jobs and verified artifact bytes')
        time.sleep(max(0, claimed['lease_until_ms'] / 1000 - time.time() + 0.1))
        recovered = server.call({'operation':'run','job':'interrupted'})
        assert recovered['attempt'] == claimed['attempt'] + 1
        assert recovered['status'] == 'failed' and recovered['artifact_sha256'] is None
        server.call({'operation':'artifact','job':'interrupted'}, 422)
        phase('expired crashed-worker claim is reclaimed with a new generation; fuel trap has no artifact')
        run(['git','fsck','--strict'], repo)
        refused = native('serve','--listen','0.0.0.0:0','--repository',SCOPE,ok=False)
        assert refused.returncode != 0
        phase('Git remains intact and non-loopback listener is rejected')
    finally:
        server.close()

print('GITR_REDB_ACTION_API_SMOKE=passed; local trusted-user profile; no MCP, remote authorization or multi-host HA claim', flush=True)
