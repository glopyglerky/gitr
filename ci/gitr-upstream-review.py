#!/usr/bin/env python3
"""Validate source-only reviews and merge them with the immutable 476-row survey.

No network, donor code execution, license approval or repository mutation occurs.
Run from the repository root; generated catalogs belong in --output, not baseline.
"""
import argparse
import copy
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import unittest
from urllib.parse import urlsplit

BASE = Path('Documentation/gitr/research/2026-11-08-upstream-survey')
BASE_SHA = '4e6710e8c625b9c8076c581430123e33ff0a4d75'
FIELDS = ['review_revision', 'review_license', 'review_inspection', 'review_components',
          'review_reuse', 'review_findings', 'review_adaptations', 'review_acceptance',
          'review_sources', 'review_execution']


def identity(url):
    value = urlsplit(url)
    if value.scheme != 'https' or not value.hostname or value.username or value.password:
        raise ValueError('expected an HTTPS project URL without credentials')
    path = value.path.rstrip('/')
    if value.hostname.lower() == 'github.com':
        parts = path.strip('/').split('/')
        if len(parts) != 2 or not all(parts):
            raise ValueError('GitHub candidate must identify a repository, not a component')
        path = '/' + '/'.join(parts).removesuffix('.git').lower()
    return value.hostname.lower() + path


def validate(document):
    if document.get('schema_version') != 1 or document.get('donor_execution') != 'not_run':
        raise ValueError('this ledger describes source inspection, not executed qualification')
    seen = set()
    for row in document['reviews']:
        repository = row['repository']
        if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
            raise ValueError('invalid repository identity')
        key = identity('https://github.com/' + repository)
        if key in seen:
            raise ValueError('duplicate repository review; list subcrates as components')
        seen.add(key)
        if not re.fullmatch(r'[0-9a-f]{40}', row['revision']):
            raise ValueError('review requires an immutable commit pin')
        if row['inspection'] not in ('selected_source', 'manifest_and_documentation'):
            raise ValueError('unsupported evidence level')
        if row['license_evidence'] not in row['inspected_paths']:
            raise ValueError('license evidence must be an actually inspected path')
        for field in ('license', 'reuse', 'findings', 'adaptations', 'acceptance', 'components'):
            if not row.get(field):
                raise ValueError('missing review field: ' + field)
        for path in row['inspected_paths']:
            if not path or path.startswith('/') or '..' in path.split('/') or '\\' in path:
                raise ValueError('invalid inspected repository path')
    return document['reviews']


def merge(baseline, document):
    reviews = validate(document)
    rows = {}
    for row in baseline:
        key = identity(row['url'])
        if key in rows:
            raise ValueError('ambiguous baseline URL identity: ' + key)
        rows[key] = dict(row)
    added, reassessed = [], []
    for review in reviews:
        url = 'https://github.com/' + review['repository']
        key = identity(url)
        if key not in rows:
            added.append(review['repository'])
            rows[key] = {'candidate_id': key, 'name': review['repository'], 'url': url,
                         'host': 'github.com', 'candidate_kind': 'repository',
                         'tracks': 'expansion', 'reports': 'expansion/README.md'}
        else:
            reassessed.append(review['repository'])
        source = url + '/blob/' + review['revision'] + '/'
        values = [review['revision'], review['license'], review['inspection'],
                  ' | '.join(review['components']), review['reuse'], review['findings'],
                  review['adaptations'], review['acceptance'],
                  ' | '.join(source + p for p in review['inspected_paths']), 'not_run']
        rows[key].update(zip(FIELDS, values))
    summary = {'baseline_candidates': len(baseline), 'reviewed_repositories': len(reviews),
               'new_candidates': len(added), 'reassessed_candidates': len(reassessed),
               'expanded_candidates': len(rows), 'new_repositories': sorted(added),
               'reassessed_repositories': sorted(reassessed), 'donor_execution': 'not_run'}
    return [rows[key] for key in sorted(rows)], summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
        raise SystemExit(not result.wasSuccessful())
    if args.output is None:
        parser.error('--output is required; baseline catalogs are never overwritten')
    raw = (BASE / 'catalog.csv').read_bytes()
    blob = hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()
    if blob != BASE_SHA:
        raise ValueError('baseline changed: review provenance before changing the expected hash')
    reader = csv.DictReader(io.StringIO(raw.decode('utf-8'), newline=''))
    baseline = list(reader)
    if len(baseline) != 476:
        raise ValueError('unexpected baseline candidate count')
    document = json.loads((BASE / 'expansion/reviews.json').read_text())
    rows, summary = merge(baseline, document)
    output = args.output.resolve()
    if output == BASE.resolve() or BASE.resolve() in output.parents:
        raise ValueError('generated output must not overwrite survey inputs')
    output.mkdir(parents=True, exist_ok=True)
    fields = list(reader.fieldnames) + FIELDS
    with (output / 'catalog.csv').open('w', newline='', encoding='utf-8') as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)
    payload = {'schema_version': 1, 'baseline_git_blob': blob, 'coverage': summary,
               'candidates': rows, 'reviews': document['reviews']}
    (output / 'catalog.json').write_text(json.dumps(payload, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    (output / 'coverage.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
    print('SURVEY_REVIEW=' + json.dumps(summary, sort_keys=True))


class Tests(unittest.TestCase):
    def fixture(self):
        return {'schema_version': 1, 'donor_execution': 'not_run', 'reviews': [
            {'repository': 'Owner/Repo', 'revision': 'a' * 40, 'license': 'MIT',
             'license_evidence': 'LICENSE', 'inspected_paths': ['LICENSE', 'src/lib.rs'],
             'inspection': 'selected_source', 'components': ['core'], 'reuse': 'whole crate',
             'findings': 'fixture only', 'adaptations': 'retain authority', 'acceptance': 'test limits'}]}

    def test_canonical_repository(self):
        self.assertEqual(identity('https://github.com/Owner/Repo.git/'), 'github.com/owner/repo')

    def test_reject_credentials_and_component_counting(self):
        for url in ['http://github.com/a/b', 'https://user@github.com/a/b', 'https://github.com/a/b/tree/main']:
            with self.assertRaises(ValueError): identity(url)

    def test_duplicate_reviews(self):
        doc = self.fixture(); doc['reviews'] *= 2
        with self.assertRaises(ValueError): validate(doc)

    def test_moving_revision_rejected(self):
        doc = self.fixture(); doc['reviews'][0]['revision'] = 'main'
        with self.assertRaises(ValueError): validate(doc)

    def test_license_evidence_required(self):
        doc = self.fixture(); doc['reviews'][0]['license_evidence'] = 'not-inspected'
        with self.assertRaises(ValueError): validate(doc)

    def test_no_source_evidence_as_execution(self):
        doc = self.fixture(); doc['donor_execution'] = 'passed'
        with self.assertRaises(ValueError): validate(doc)

    def test_missing_acceptance_rejected(self):
        doc = self.fixture(); doc['reviews'][0]['acceptance'] = ''
        with self.assertRaises(ValueError): validate(doc)

    def test_existing_evidence_is_retained(self):
        base = [{'url': 'https://github.com/owner/repo', 'original_unknown_column': 'unchanged'}]
        before = copy.deepcopy(base)
        rows, stats = merge(base, self.fixture())
        self.assertEqual(rows[0]['original_unknown_column'], 'unchanged')
        self.assertEqual(stats['reassessed_candidates'], 1)
        self.assertEqual(stats['new_candidates'], 0)
        self.assertEqual(base, before)

    def test_new_candidate_and_repeatable_output(self):
        rows, stats = merge([], self.fixture())
        self.assertEqual(stats['expanded_candidates'], 1)
        self.assertEqual(stats['new_candidates'], 1)
        self.assertEqual((rows, stats), merge([], self.fixture()))

    def test_ambiguous_baseline_rejected(self):
        base = [{'url': 'https://github.com/a/b'}, {'url': 'https://github.com/A/B/'}]
        with self.assertRaises(ValueError): merge(base, self.fixture())


if __name__ == '__main__':
    main()
