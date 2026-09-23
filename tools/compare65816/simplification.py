#!/usr/bin/env python3
"""Authenticate the native work exporter and prepare its size-ladder CLI manifest."""
import argparse
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def summarize(directory, compiler):
    rows = json.loads((directory / 'work.json').read_text())
    assert len(rows) == 44 and len({(r['case'], r['mode']) for r in rows}) == 44
    stress = [r for r in rows if r['stress'] is not None]
    assert len(stress) == 16
    artifacts = []
    totals = {}
    for row in rows:
        for name in ('source', 'image'):
            row[name + '_sha256'] = digest(directory / row[name])
        assert row['mir_operations'] > 0 and row['code_bytes'] > 0
        if row['stress'] is None:
            for kind, count in row['work'].items():
                totals[kind] = totals.get(kind, 0) + count
        else:
            assert row['boundary_executions'] == 10 and row['crlf_equal']
            assert row['mir_operations'] >= row['stress'][1]
            command = [str(compiler.resolve()), '--layout', str((directory / 'layout.json').resolve()),
                       '--module-path', str(Path('runtime/65816').resolve()),
                       '-o', str((directory / row['image']).resolve()),
                       str((directory / row['source']).resolve())]
            if row['mode'] == 'raw':
                command.insert(1, '--no-opt')
            artifacts.append(dict(compiler='actionc', case=row['case'], mode=row['mode'],
                                  hashes={'image.json': row['image_sha256']}, commands=[command]))
    # Both modes of each family must retain increasing emitted work.
    for family in ('chain', 'branches'):
        for mode in ('raw', 'optimized'):
            sequence = sorted((r for r in stress if r['stress'][0] == family and r['mode'] == mode),
                              key=lambda r: r['stress'][1])
            assert all(a['code_bytes'] < b['code_bytes'] for a, b in zip(sequence, sequence[1:]))
    return dict(builds=rows, corpus_work=totals, inputs={
        'work.json': digest(directory / 'work.json'), 'layout.json': digest(directory / 'layout.json')}), dict(artifacts=artifacts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    summary, manifest = summarize(args.directory, args.compiler)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2) + '\n')
    (args.directory / 'host-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(summary['corpus_work'], indent=2))


if __name__ == '__main__':
    main()
