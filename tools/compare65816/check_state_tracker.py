#!/usr/bin/env python3
"""Require complete code and measurement equality for the state-tracker refactor."""
import argparse
import hashlib
import json
from pathlib import Path
from delta import load


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def equal_records(old, new):
    assert old.keys() == new.keys(), 'measurement keys'
    for key in old:
        assert old[key] == new[key], ('measurement changed', key)


def equal_artifact(old, new):
    # Only build provenance may differ; executable/map contracts must match.
    excluded = {'directory', 'commands', 'hashes'}
    assert {k: v for k, v in old.items() if k not in excluded} == {
        k: v for k, v in new.items() if k not in excluded}, 'artifact contract'
    assert old['hashes'].keys() == new['hashes'].keys(), 'artifact files'
    for name in old['hashes']:
        contents = []
        for artifact in (old, new):
            path = Path(artifact['directory']) / name
            assert digest(path) == artifact['hashes'][name], ('artifact hash', path)
            contents.append(path.read_bytes())
        if old['compiler'] == 'vbcc' and name == 'code.lst':
            headers = [f'Source: "{a["directory"]}/code.asm"'.encode() for a in (old, new)]
            assert all(data.count(header) == 1 for data, header in zip(contents, headers))
            contents[0] = contents[0].replace(headers[0], headers[1])
        assert contents[0] == contents[1], ('artifact bytes', old['case'], name)


def check(before, after, baseline):
    for field in ('qualification_record', 'measurements', 'snapshot_hashes'):
        for path, expected in baseline[field].items():
            assert digest(Path(path)) == expected, ('frozen baseline', path)
    old_manifest, old = load(before)
    new_manifest, new = load(after)
    equal_records(old, new)
    for field in ('schema', 'target', 'cases'):
        assert old_manifest[field] == new_manifest[field], field
    for tool in ('vbcc', 'vasm', 'vlink'):
        assert old_manifest['tools'][tool]['sha256'] == new_manifest['tools'][tool]['sha256'], tool
    def index(manifest):
        result = {(a['case'], a['mode'], a['compiler']): a for a in manifest['artifacts']}
        assert len(result) == len(manifest['artifacts']), 'duplicate build'
        return result
    originals, builds = index(old_manifest), index(new_manifest)
    assert originals.keys() == builds.keys(), 'build keys'
    for key in originals:
        equal_artifact(originals[key], builds[key])
    return dict(records=len(new), builds=len(builds), artifact_files=sum(len(a['hashes']) for a in builds.values()),
                complete_records_equal=True, artifacts_equal=True,
                external_failures=[dict(zip(('case', 'mode', 'compiler', 'vector'), k))
                                   for k, v in new.items() if not v['correct']])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.before, args.after, json.loads(args.baseline.read_text()))
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
