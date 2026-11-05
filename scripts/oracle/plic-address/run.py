#!/usr/bin/env python3
"""Real PLIC source trace parity; host buffers are not MMIO/SMP/runtime proof."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
BASE_SHA = 'b0c4fa1dcc5c556f9a7c42955fabb992b6d5d0e8f6814387a4187ea7499a66e1'
IO_SHA = {
    'mod.rs': 'f2dc450fdb7409b0d909157ca71f98e2ed27d525ab638e7d169e03e334bad319',
    'mmio.rs': 'f067b339e7e74c97f65cdf3511a12322b7ed989f16b719672d10ba603435ec63',
    'mmio_ptr.rs': '5534a8e119b00173e33824e894f749137e7e6f692a1585be608b735333685f76',
}
CONTROLS = {
    'wrong-context-stride': 'oracle_context_stride',
    'extra-claim-read': 'oracle_claim_read_once_and_zero_claim_panics',
    'wrong-enable-bit': 'oracle_enable_disable_priority_then_single_rmw',
    'rmw-before-priority': 'oracle_enable_disable_priority_then_single_rmw',
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kernel', type=Path)
    parser.add_argument('evidence', type=Path, help='new directory; refuses reuse')
    args = parser.parse_args()
    root, out = args.kernel.resolve(), args.evidence.resolve()
    current = root / 'src/arch/riscv64/device/irqchip/plic.rs'
    io = root / 'syscall/src/io'
    assert digest(HERE / 'before.rs') == BASE_SHA, 'baseline fixture changed'
    for name, expected in IO_SHA.items():
        assert digest(io / name) == expected, 'actual Io source changed; review required'
    out.mkdir(parents=True, exist_ok=False)
    crate = out / 'harness'
    (crate / 'src').mkdir(parents=True)
    (crate / 'tests/common').mkdir(parents=True)
    for src, dst in [('Cargo.toml.in', 'Cargo.toml'), ('lib.rs.in', 'src/lib.rs'),
                     ('hook.rs', 'src/hook.rs'), ('parity.rs', 'tests/parity.rs'),
                     ('cases.rs', 'tests/common/mod.rs')]:
        shutil.copyfile(HERE / src, crate / dst)
    env = dict(os.environ, CARGO_NET_OFFLINE='true', RUSTUP_TOOLCHAIN='nightly-2026-05-24',
               CARGO_TARGET_DIR=str(out / 'target'), CARGO_BUILD_JOBS='2')
    env['PATH'] = '$HOME/.cargo/bin:' + env['PATH']
    results = []

    def run(label, argv, extra=None):
        with (out / (label + '.log')).open('w') as log:
            result = subprocess.run(argv, cwd=crate, env=dict(env, **(extra or {})),
                                    stdout=log, stderr=subprocess.STDOUT, timeout=120)
        text = (out / (label + '.log')).read_text()
        results.append({'label': label, 'exit': result.returncode})
        return result.returncode, text

    def generate(label, source, control=None):
        gen = out / label
        gen.mkdir()
        (gen / 'traces').mkdir()
        commands = [('io', io, gen / 'io.rs'), ('plic', HERE / 'before.rs', gen / 'old.rs'),
                    ('plic', source, gen / 'new.rs')]
        for index, command in enumerate(commands):
            argv = ['python3', str(HERE / 'extract.py'), *map(str, command)]
            if index == 2 and control:
                argv += ['--control', control]
            code, _ = run(label + '-extract-' + str(index), argv)
            assert code == 0, 'extraction failed: ' + label
        return {'PLIC_GEN_DIR': str(gen), 'PLIC_TRACE_DIR': str(gen / 'traces')}

    for label, source in [('baseline', HERE / 'before.rs'), ('current', current)]:
        extra = generate(label, source)
        code, text = run(label, ['cargo', 'test', '--offline', '--', '--test-threads=1', '--nocapture'], extra)
        assert code == 0 and '8 passed; 0 failed' in text, label + ' parity failed'
    for control, expected in CONTROLS.items():
        extra = generate(control, current, control)
        code, _ = run(control + '-compile', ['cargo', 'test', '--offline', '--no-run'], extra)
        assert code == 0, 'control did not compile: ' + control
        code, text = run(control, ['cargo', 'test', '--offline', '--', '--test-threads=1', '--nocapture'], extra)
        assert code == 101 and re.search(r'^    ' + re.escape(expected) + r'$', text, re.M) and 'assertion' in text, control
    for version in ['old', 'new']:
        extra = generate('traits-' + version, current)
        for trait in ['send', 'sync']:
            cfg = 'plic_assert_' + trait + '_' + version
            code, text = run(cfg, ['cargo', 'build', '--offline', '--lib'],
                             dict(extra, RUSTFLAGS='--cfg ' + cfg))
            assert code != 0 and 'E0277' in text and '*mut' in text, cfg
    extra = generate('drop-marker', current, 'drop-marker')
    for trait in ['send', 'sync']:
        code, _ = run('marker-control-' + trait, ['cargo', 'build', '--offline', '--lib'],
                      dict(extra, RUSTFLAGS='--cfg plic_assert_' + trait + '_new'))
        assert code == 0, 'trait fixture not detecting marker: ' + trait
    receipt = {'source_sha256': digest(current), 'baseline_sha256': BASE_SHA,
               'scope': __doc__, 'results': results, 'pass': True}
    (out / 'ACCEPTANCE.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
