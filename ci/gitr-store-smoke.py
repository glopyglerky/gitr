#!/usr/bin/env python3
"""Real SurrealKV owner, independent native clients, and actual Rust/Wasm jobs.

All data and credentials are temporary. No user repository or external service
is used. Git builds the fixture; Gitr reads snapshots and executes guests itself.
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
import time
import urllib.error
import urllib.request

ROOT = Path.cwd()
GITR = (ROOT / 'rust/gitr/target/debug/gitr').resolve()
STORE = (ROOT / 'rust/gitr-store/target/debug/gitr-store').resolve()
WASM = (ROOT / 'rust/gitr-action-example/target/wasm32-unknown-unknown/release/gitr_action_example.wasm').resolve()
TOKEN = secrets.token_hex(32)
ENV = {**os.environ, 'GITR_STORE_TOKEN': TOKEN}
REPOSITORY = 'qualification'
PROFILE = 'gitr-actions-v1/wasmi-2.0.0/fuel-10000000/memory-16MiB/io-8MiB/calls-4096'


def command(argv, cwd, ok=True, timeout=40, env=None):
    result = subprocess.run(argv, cwd=cwd, env=env or ENV, capture_output=True,
                            text=True, timeout=timeout)
    if ok and result.returncode:
        raise AssertionError(f'{Path(argv[0]).name}: {result.stderr[:2000]}')
    return result


class Owner:
    def __init__(self, directory):
        self.error_file = tempfile.TemporaryFile(mode='w+')
        self.process = subprocess.Popen([str(STORE), str(directory), '127.0.0.1:0', REPOSITORY],
                                        env=ENV, stdout=subprocess.PIPE, stderr=self.error_file,
                                        text=True)
        lines = queue.Queue()
        threading.Thread(target=lambda: lines.put(self.process.stdout.readline()), daemon=True).start()
        try:
            line = lines.get(timeout=60)
            self.ready = json.loads(line)
            self.endpoint = self.ready['endpoint']
            assert self.ready['repository'] == REPOSITORY
        except Exception:
            self.stop()
            self.error_file.seek(0)
            raise AssertionError('storage owner startup failed: ' + self.error_file.read()[:3000])

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=15)
        self.process.stdout.close()

    def close(self):
        self.stop()
        self.error_file.close()

    def call(self, operation, expected=200, token=TOKEN, repository=REPOSITORY,
             extra_headers=None, raw=None, content_type='application/json'):
        headers = {'Content-Type': content_type, 'x-gitr-repository': repository}
        if token is not None:
            headers['Authorization'] = 'Bearer ' + token
        if extra_headers:
            headers.update(extra_headers)
        payload = raw if raw is not None else json.dumps(operation).encode()
        request = urllib.request.Request(self.endpoint, data=payload, headers=headers, method='POST')
        # Ignore ambient proxies: the service is literal loopback only.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(request, timeout=30) as response:
                status, data = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, data = error.code, error.read()
        assert status == expected, (operation.get('operation'), expected, status, data[:500])
        assert TOKEN.encode() not in data
        return json.loads(data)


def phase(name):
    print('PASS ' + name, flush=True)


with tempfile.TemporaryDirectory(prefix='gitr-store-proof-') as temporary:
    temp = Path(temporary)
    repo = temp / 'repository'
    repo.mkdir()
    state = temp / 'state'
    command(['git', 'init', '-q'], repo)
    command(['git', 'config', 'user.name', 'Gitr qualification'], repo)
    command(['git', 'config', 'user.email', 'gitr@example.invalid'], repo)
    command(['git', 'config', 'core.autocrlf', 'false'], repo)
    (repo / 'source.rs').write_bytes(b'fn main() {}\n')
    command(['git', 'add', 'source.rs'], repo)
    command(['git', 'commit', '-qm', 'immutable source'], repo)
    owner = Owner(state)
    try:
        def native(*args, ok=True):
            return command([str(GITR), 'actions', 'remote', '--endpoint', owner.endpoint,
                            '--repository', REPOSITORY, *args], repo, ok=ok)

        read = {'operation': 'status', 'job': 'unknown'}
        owner.call(read, 401, token=None)
        owner.call(read, 401, token='0' * 64)
        owner.call(read, 403, repository='other')
        owner.call(read, 403, extra_headers={'Origin': 'https://untrusted.invalid'})
        owner.call(read, 415, content_type='text/plain')
        owner.call(read, 404)
        owner.call({**read, 'sql': 'DELETE gitr_job'}, 400)
        owner.call({'operation': 'status', 'job': '../outside'}, 400)
        owner.call(read, 413, raw=b'x' * (2 * 1024 * 1024 + 32769))
        phase('authentication, repository scope, denied browser origin, bounded typed requests')

        module = json.loads(native('install', str(WASM)).stdout)['module_sha256']
        assert module == hashlib.sha256(WASM.read_bytes()).hexdigest()
        (repo / 'source.rs').write_bytes(b'dirty text with trailing spaces  \n')
        submitted = json.loads(native('submit', 'policy', module, '--path', 'source.rs').stdout)
        assert submitted['spec']['input_sha256'] == hashlib.sha256(b'fn main() {}\n').hexdigest()
        passed = json.loads(native('run', 'policy').stdout)
        assert passed['status'] == 'passed' and passed['attempt'] == 1
        assert json.loads(native('artifact', 'policy').stdout) == {
            'bytes': 13, 'lines': 1, 'trailing_whitespace': False}
        assert json.loads(native('run', 'policy').stdout) == passed
        assert json.loads(native('submit', 'policy', module, '--path', 'source.rs').stdout) == passed
        assert not (repo / '.git/gitr/state.redb').exists()
        phase('real native CLI and Rust/Wasm action use SurrealDB without opening local redb')

        data = b'fn main() {}\n'
        spec = submitted['spec']
        def submit(job, payload=data, specification=None):
            return owner.call({'operation': 'submit', 'job': job, 'spec': specification or spec,
                               'input_hex': payload.hex()})

        submit('race')
        def claim(_):
            return owner.call({'operation': 'claim', 'job': 'race'})['claim']
        with ThreadPoolExecutor(max_workers=4) as pool:
            claims = list(pool.map(claim, range(4)))
        winners = [c for c in claims if c is not None]
        assert len(winners) == 1, claims
        winner = winners[0]
        # Create another job while the first is claimed. No guest-duration writer lock.
        submit('independent')
        assert json.loads(native('run', 'independent').stdout)['status'] == 'passed'
        output = b'atomic artifact'
        finish = {'operation': 'finish', 'job': 'race', 'attempt': winner['attempt'],
                  'exit_code': 0, 'output_hex': output.hex()}
        completed = owner.call(finish)
        assert completed['artifact_sha256'] == hashlib.sha256(output).hexdigest()
        assert owner.call(finish) == completed
        owner.call({**finish, 'output_hex': b'different'.hex()}, 409)
        assert owner.call({'operation': 'artifact', 'job': 'race'})['hex'] == output.hex()
        phase('concurrent claims have one winner; artifact and receipt persist; identical finish retries')

        changed = {**spec, 'input_sha256': hashlib.sha256(b'other').hexdigest()}
        owner.call({'operation': 'submit', 'job': 'policy', 'spec': changed, 'input_hex': b'other'.hex()}, 409)
        with ThreadPoolExecutor(max_workers=4) as pool:
            repeated = list(pool.map(lambda _: submit('same-key'), range(4)))
        assert all(row == repeated[0] for row in repeated)
        phase('submission idempotency and conflicting payload rejection')

        submit('cancelled')
        cancelled_claim = owner.call({'operation': 'claim', 'job': 'cancelled'})['claim']
        owner.call({'operation': 'cancel', 'job': 'cancelled'})
        forbidden = b'never-publish-cancelled'
        owner.call({'operation': 'finish', 'job': 'cancelled', 'attempt': cancelled_claim['attempt'],
                    'exit_code': 0, 'output_hex': forbidden.hex()}, 409)
        owner.call({'operation': 'object', 'sha256': hashlib.sha256(forbidden).hexdigest()}, 404)
        assert owner.call({'operation': 'claim', 'job': 'cancelled'})['claim'] is None
        assert owner.call({'operation': 'cancel', 'job': 'policy'}) == passed
        phase('cancellation fences old worker and publishes no partial artifact')

        command(['git', 'add', 'source.rs'], repo)
        command(['git', 'commit', '-qm', 'policy violation'], repo)
        native('submit', 'bad-policy', module, '--path', 'source.rs')
        failed = native('run', 'bad-policy', ok=False)
        assert failed.returncode != 0 and json.loads(failed.stdout)['status'] == 'failed'
        assert native('artifact', 'bad-policy', ok=False).returncode != 0
        phase('actual failing guest returns failure and no artifact')

        # Use a real 60-second expiry, not a caller-supplied clock or bypass.
        submit('expired')
        old = owner.call({'operation': 'claim', 'job': 'expired'})['claim']
        second = command([str(STORE), str(state), '127.0.0.1:0', REPOSITORY], repo, ok=False)
        assert second.returncode != 0 and 'lock' in second.stderr.lower(), second.stderr
        phase('second SurrealKV owner is rejected')
        # Forced termination with acknowledged receipts already on disk.
        owner.close()
        owner = Owner(state)
        assert json.loads(native('status', 'policy').stdout) == passed
        assert owner.call({'operation': 'status', 'job': 'race'}) == completed
        assert json.loads(native('artifact', 'policy').stdout)['bytes'] == 13
        phase('acknowledged action receipts and artifact survive forced owner termination')
        remaining = max(0, old['lease_until_ms'] / 1000 - time.time() + 0.1)
        time.sleep(remaining)
        expired_output = b'expired-output-must-not-exist'
        owner.call({'operation': 'finish', 'job': 'expired', 'attempt': old['attempt'],
                    'exit_code': 0, 'output_hex': expired_output.hex()}, 409)
        owner.call({'operation': 'object', 'sha256': hashlib.sha256(expired_output).hexdigest()}, 404)
        newer = owner.call({'operation': 'claim', 'job': 'expired'})['claim']
        assert newer['attempt'] == old['attempt'] + 1
        owner.call({'operation': 'finish', 'job': 'expired', 'attempt': old['attempt'],
                    'exit_code': 0, 'output_hex': expired_output.hex()}, 409)
        owner.call({'operation': 'finish', 'job': 'expired', 'attempt': newer['attempt'],
                    'exit_code': 0, 'output_hex': b'recovered'.hex()})
        phase('expired and replaced claims cannot publish; crashed work recovers with a new generation')
        command(['git', 'fsck', '--strict'], repo)
        phase('ordinary Git repository remains intact')
    finally:
        owner.close()

    nonempty = temp / 'not-gitr'
    nonempty.mkdir(mode=0o700)
    (nonempty / 'sentinel').write_text('preserve')
    refused = command([str(STORE), str(nonempty), '127.0.0.1:0', REPOSITORY], repo, ok=False)
    assert refused.returncode != 0 and (nonempty / 'sentinel').read_text() == 'preserve'
    assert not (nonempty / 'database').exists()
    wrong = command([str(STORE), str(state), '127.0.0.1:0', 'wrong-repository'], repo, ok=False)
    assert wrong.returncode != 0
    phase('unmarked existing directories and mismatched repository identities are never adopted')

print('GITR_STORE_SMOKE=passed; local trusted-user profile only; no migration, MCP, multi-tenant authorization or HA claim', flush=True)
