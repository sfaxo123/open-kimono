#!/usr/bin/env python3
"""Convert mapped Linux rwmmio events to mmio-replay physical-address JSONL."""
import argparse
import json
import re
import sys
from decimal import Decimal
from pathlib import Path

EVENT = re.compile(r'\brwmmio_(read|post_read|write|post_write):\s*(.*)')
FIELD = re.compile(r'\b(caller|width|val|addr)=([^\s]+)')
STAMP = re.compile(r'\s(\d+\.\d+):\s+rwmmio_')
WIDTHS = {1: 8, 2: 16, 4: 32, 8: 64}


def convert(lines, virt_base, phys_base, size, caller_pattern=None):
    """Yield replay records; only post-read and pre-write contain unique accesses."""
    caller_re = re.compile(caller_pattern) if caller_pattern else None
    previous_ns = None
    for line_number, line in enumerate(lines, 1):
        match = EVENT.search(line)
        if not match or match.group(1) not in ('post_read', 'write'):
            continue
        fields = dict(FIELD.findall(match.group(2)))
        if not {'width', 'val', 'addr'} <= fields.keys():
            raise ValueError(f'line {line_number}: incomplete rwmmio event')
        caller = fields.get('caller', '')
        if caller_re and not caller_re.search(caller):
            continue
        try:
            width_raw = int(fields['width'], 0)
            width = WIDTHS.get(width_raw, width_raw)
            addr = int(fields['addr'], 0)
            value = int(fields['val'], 0)
        except ValueError as exc:
            raise ValueError(f'line {line_number}: invalid field: {exc}') from exc
        if width not in (8, 16, 32, 64):
            raise ValueError(f'line {line_number}: unsupported width {width_raw}')
        if not virt_base <= addr < virt_base + size or addr + width // 8 > virt_base + size:
            continue
        stamp = STAMP.search(line)
        now_ns = int(Decimal(stamp.group(1)) * 1_000_000_000) if stamp else None
        delay_us = max(0, (now_ns - previous_ns) // 1000) if now_ns is not None and previous_ns is not None else 0
        previous_ns = now_ns
        yield {'op': 'r' if match.group(1) == 'post_read' else 'w', 'width': width,
               'addr': hex(phys_base + addr - virt_base),
               'value': hex(value & ((1 << width) - 1)), 'delay_us': delay_us}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('jsonl', type=Path)
    parser.add_argument('--virtual-base', type=lambda s: int(s, 0), required=True)
    parser.add_argument('--physical-base', type=lambda s: int(s, 0), required=True)
    parser.add_argument('--size', type=lambda s: int(s, 0), required=True)
    parser.add_argument('--caller', help='optional regex selecting a Linux driver caller')
    parser.add_argument('--dry-run', action='store_true', help='validate and count without writing JSONL')
    args = parser.parse_args()
    if args.size <= 0 or args.virtual_base < 0 or args.physical_base < 0:
        parser.error('mapping bases must be nonnegative and size positive')
    if args.raw.resolve() == args.jsonl.resolve():
        parser.error('raw and JSONL paths must differ')
    with args.raw.open(encoding='utf-8', errors='replace') as source:
        records = list(convert(source, args.virtual_base, args.physical_base, args.size, args.caller))
    if not records:
        parser.error('no mapped rwmmio events; check tracepoint availability and VA-to-PA mapping')
    if not args.dry_run:
        with args.jsonl.open('w', encoding='utf-8') as dest:
            for record in records:
                dest.write(json.dumps(record, separators=(',', ':')) + '\n')
    print(f'{len(records)} MMIO accesses -> {args.jsonl}' + (' (dry run)' if args.dry_run else ''), file=sys.stderr)


if __name__ == '__main__':
    main()
