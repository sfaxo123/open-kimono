#!/usr/bin/env python3
"""Source-rooted GICv3 diagnostic parity. Host shims are not CPU/MMIO proof."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
MANIFEST = '[package]\nname="gicv3-rd-harness"\nversion="0.0.0"\nedition="2021"\n[lib]\npath="src/lib.rs"\n[workspace]\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kernel', type=Path)
    parser.add_argument('evidence', type=Path, help='new output directory')
    args = parser.parse_args()
    root, out = args.kernel.resolve(), args.evidence.resolve()
    source = root / 'src/arch/aarch64/device/irqchip/gicv3.rs'
    model = source.with_suffix('') / 'model.rs'
    assert source.is_file() and model.is_file(), 'wrong kernel source root'
    out.mkdir(parents=True, exist_ok=False)
    crate = out / 'harness'
    (crate / 'src').mkdir(parents=True)
    (crate / 'Cargo.toml').write_text(MANIFEST)
    for name in ('gen.py', 'run_all.sh', 'before.rs'):
        shutil.copyfile(HERE / name, crate / name)
    for name in ('shim.rs', 'parity_tests.rs', 'current_tests.rs'):
        shutil.copyfile(HERE / name, crate / 'src' / name)
    text = (HERE / 'lib.rs').read_text()
    old = '__MODEL_SOURCE__'
    assert text.count(old) == 1, 'model source anchor changed'
    (crate / 'src/lib.rs').write_text(text.replace(json.dumps(old), json.dumps(str(model))))
    env = dict(os.environ, KERNEL_ROOT=str(root), GICV3_RD_EVIDENCE=str(out),
               CARGO_NET_OFFLINE='true', RUSTUP_TOOLCHAIN='nightly-2026-05-24')
    env['PATH'] = '$HOME/.cargo/bin:' + env['PATH']
    with (out / 'run.log').open('w') as log:
        result = subprocess.run(['bash', str(crate / 'run_all.sh')], env=env,
                                stdout=log, stderr=subprocess.STDOUT, timeout=120)
    receipt = {'exit': result.returncode, 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
               'scope': 'real source host parity and behavioral controls only; no CPU/MMIO/runtime proof'}
    (out / 'ACCEPTANCE.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print((out / 'run.log').read_text())
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
