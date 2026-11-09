#!/usr/bin/env python3
"""Fixed-fixture startup/runtime/RSS comparison; no universal speed claim."""
import argparse, hashlib, json, os, pathlib, random, statistics, subprocess, tempfile, time
p = argparse.ArgumentParser()
p.add_argument('--before', required=True)
p.add_argument('--after', required=True)
p.add_argument('--output', required=True)
a = p.parse_args()
bins = {'git': '/usr/bin/git', 'before': str(pathlib.Path(a.before).resolve()), 'after': str(pathlib.Path(a.after).resolve())}
env = {**os.environ, 'LC_ALL': 'C', 'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_AUTHOR_DATE': '2026-01-02T03:04:05Z', 'GIT_COMMITTER_DATE': '2026-01-02T03:04:05Z'}
with tempfile.TemporaryDirectory() as d:
    root = pathlib.Path(d)
    def git(*args): return subprocess.check_output([bins['git'], *args], cwd=root, env=env, stderr=subprocess.DEVNULL)
    git('init', '-q', '-b', 'main')
    git('config', 'user.name', 'Benchmark')
    git('config', 'user.email', 'benchmark@example.invalid')
    for i in range(1000):
        path = root / f'dir{i % 20:02}/file{i:04}.txt'
        path.parent.mkdir(exist_ok=True)
        path.write_bytes((f'fixed record {i}\n' * 64).encode())
    (root / 'payload').write_bytes(bytes(range(256)) * 16384)
    git('add', '.')
    git('commit', '-qm', 'Fixed benchmark fixture')
    tree = git('rev-parse', 'HEAD^{tree}').decode().strip()
    ops = {'rev-parse': ['rev-parse', 'HEAD'], 'status': ['status', '--porcelain', '--no-renames'], 'hash-object': ['hash-object', '--no-filters', 'payload'], 'ls-tree': ['ls-tree', '-r', '-z', 'HEAD']}
    rows = []
    for op, args in ops.items():
        names = ['git', 'before', 'after'] if op != 'ls-tree' else ['git', 'after']
        expected = subprocess.check_output([bins['git'], *args], cwd=root, env=env)
        for trial in range(24):
            order = names[trial % len(names):] + names[:trial % len(names)]
            for name in order:
                start = time.perf_counter_ns()
                with tempfile.TemporaryFile() as capture:
                    prior = os.getcwd()
                    os.chdir(root)
                    try:
                        pid = os.posix_spawn(bins[name], [bins[name], *args], env, file_actions=[(os.POSIX_SPAWN_DUP2, capture.fileno(), 1)])
                        _, status, usage = os.wait4(pid, 0)
                        assert os.waitstatus_to_exitcode(status) == 0
                    finally:
                        os.chdir(prior)
                    capture.seek(0)
                    out = capture.read()
                elapsed = time.perf_counter_ns() - start
                assert out == expected, (op, name)
                rows.append({'operation': op, 'trial': trial, 'binary': name, 'elapsed_ns': elapsed, 'wait4_maxrss_kib_including_launcher': usage.ru_maxrss, 'output_sha256': hashlib.sha256(out).hexdigest()})
    memory_rows = []
    for op, args in ops.items():
        for name in (['git', 'after'] if op == 'ls-tree' else ['git', 'before', 'after']):
            for trial in range(5):
                samples = []
                with tempfile.TemporaryFile() as capture:
                    child = subprocess.Popen([bins[name], *args], cwd=root, env=env, stdout=capture)
                    while child.poll() is None:
                        try:
                            proc = pathlib.Path('/proc') / str(child.pid)
                            if (proc / 'exe').resolve() == pathlib.Path(bins[name]).resolve():
                                for line in (proc / 'status').read_text().splitlines():
                                    if line.startswith('VmHWM:'):
                                        samples.append(int(line.split()[1]))
                        except (FileNotFoundError, ProcessLookupError):
                            pass
                        time.sleep(0.0005)
                    assert child.returncode == 0
                memory_rows.append({'operation': op, 'binary': name, 'trial': trial, 'sampled_peak_rss_kib_lower_bound': max(samples) if samples else None})
    summary = {}
    rng = random.Random(9183)
    for op in ops:
        group = [r for r in rows if r['operation'] == op and r['trial'] >= 3]
        summary[op] = {}
        for name in sorted({r['binary'] for r in group}):
            selected = [r for r in group if r['binary'] == name]
            summary[op][name] = {'median_ns': statistics.median(r['elapsed_ns'] for r in selected)}
        native = [r['elapsed_ns'] for r in group if r['binary'] == 'after']
        ref = [r['elapsed_ns'] for r in group if r['binary'] == 'git']
        ratios = [x/y for x,y in zip(native,ref)]
        boot = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000))
        summary[op]['paired_after_over_git'] = {'median': statistics.median(ratios), 'bootstrap_95_percent': [boot[250], boot[9749]]}
    result = {'fixture_tree': tree, 'fixture': '1000 deterministic text files in 20 dirs plus 4 MiB byte-pattern payload; one fixed-date commit', 'profile': 'Gitr dev/unoptimized before and after; system Git optimized; startup and Python process-launch overhead included; warm cache; trials 0-2 excluded from summary; no compiler speed comparison', 'reference': subprocess.check_output([bins['git'], '--version']).decode().strip(), 'binaries': {n: hashlib.sha256(pathlib.Path(b).read_bytes()).hexdigest() for n,b in bins.items()}, 'commands': ops, 'memory_method': 'Separate 0.5ms /proc VmHWM samples after executable identity check; sampled lower bounds, short processes can be missed. wait4 RSS includes inherited Python high-water and must not rank memory.', 'memory_rows': memory_rows, 'rows': rows, 'summary': summary}
    pathlib.Path(a.output).write_text(json.dumps(result, indent=2)+'\n')
