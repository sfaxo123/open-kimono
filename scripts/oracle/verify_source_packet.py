#!/usr/bin/env python3
"""Match Rust code fences verbatim; prose, claimed ranges and safety are unverified."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


def check_context(packet, handoff):
    if handoff is None:
        return None
    path = handoff.resolve()
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    paths = re.findall(rb'^Canonical handoff: (.+)$', packet, re.M)
    hashes = re.findall(rb'^Canonical handoff SHA256: (.+)$', packet, re.M)
    passed = paths == [str(path).encode()] and hashes == [digest.encode()]
    return {'pass': passed, 'path': str(path), 'sha256': digest,
            'scope': 'Declared context only; does not prove the author read it.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--handoff', type=Path, help='require exact canonical handoff path/hash declarations')
    parser.add_argument('kernel', type=Path)
    parser.add_argument('packet', type=Path)
    parser.add_argument('sources', nargs='+', help='explicit relative source paths')
    args = parser.parse_args()
    root = args.kernel.resolve()
    sources = {}
    for name in args.sources:
        path = (root / name).resolve()
        relative = str(path.relative_to(root))
        if relative in sources:
            parser.error('duplicate source: ' + relative)
        sources[relative] = path.read_bytes()
    packet = args.packet.read_bytes()
    blocks = re.findall(rb'```rust\r?\n(.*?)\r?\n```', packet, re.S)
    results = []
    for number, block in enumerate(blocks, 1):
        hits = []
        for name, text in sources.items():
            for match in re.finditer(re.escape(block), text):
                start = text[:match.start()].count(b'\n') + 1
                hits.append({'source': name, 'start': start,
                             'end': start + block.count(b'\n'),
                             'source_sha256': hashlib.sha256(text).hexdigest()})
        results.append({'block': number, 'matches': hits, 'unique': len(hits) == 1})
    context = check_context(packet, args.handoff)
    code_passed = bool(blocks) and all(item['unique'] for item in results)
    passed = code_passed and (context is None or context['pass'])
    print(json.dumps({'pass': passed, 'packet_sha256': hashlib.sha256(packet).hexdigest(),
                      'blocks': results, 'context': context, 'scope': __doc__}, indent=2))
    if context is not None and not context['pass']:
        print('REJECT: missing, duplicate or stale canonical handoff declaration', file=sys.stderr)
    if not code_passed:
        failed = [str(item['block']) for item in results if not item['unique']]
        print('REJECT: missing or ambiguous verbatim source blocks: ' +
              (', '.join(failed) if failed else 'no Rust fences'), file=sys.stderr)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
