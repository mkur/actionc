#!/usr/bin/env python3
"""Require unchanged executable artifacts, all measurements and control proofs."""
import argparse
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from check_state_tracker import check as check_equality, digest

KNOWN_FAILURE = ['unlink', 'optimized', 'vbcc', 0]


def check(before, after, baseline):
    # Bind the actual comparison input to the frozen evidence, not merely to
    # some other files named by that evidence record.
    frozen = Path(baseline['comparison_directory'])
    for name in ('manifest.json', 'debug.json', 'release.json'):
        assert digest(before/name) == baseline['measurements'][str(frozen/name)], ('baseline input', name)
    assert baseline['required_external_failures'] == [KNOWN_FAILURE]
    result = check_equality(before, after, baseline)
    old = json.loads((before/'debug.json').read_text())
    new = json.loads((after/'debug.json').read_text())
    # Manifest provenance may differ. Everything else, including new/unknown
    # observer fields and complete ordered control records, must be equal.
    assert {k: v for k, v in old.items() if k != 'manifest'} == {
        k: v for k, v in new.items() if k != 'manifest'
    }, 'observer records, order or control-flow proofs changed'
    failures = [[r[k] for k in ('case', 'mode', 'compiler', 'vector')]
                for r in new['measurements'] if not r['correct']]
    assert failures == [KNOWN_FAILURE], ('external failures', failures)
    result['complete_control_and_observer_records_equal'] = True
    result['frozen_comparison_authenticated'] = True
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = check(args.before, args.after, json.loads(args.baseline.read_text()))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
