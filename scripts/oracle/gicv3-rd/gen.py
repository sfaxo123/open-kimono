#!/usr/bin/env python3
"""Extract the real private_rd items from gicv3.rs (accepted26 fixture + working tree)
into a host harness. Optional --mutant applies one negative-control edit to the
extracted CURRENT text only. Writes provenance.json for the clean run."""
import argparse, hashlib, json, os, re, subprocess, sys
from pathlib import Path

WORKTREE = Path(os.environ["KERNEL_ROOT"])
REL = "src/arch/aarch64/device/irqchip/gicv3.rs"
MODEL_REL = "src/arch/aarch64/device/irqchip/gicv3/model.rs"
BASE_REV = "36faf9cfac07d5c2552f1dc03be79245cdf11f64"
HERE = Path(__file__).resolve().parent
GEN = HERE / "src" / "generated"

MUTANTS = {
    # helper gains a log call on the miss path
    "helper_logs": ("helper",
        "Err(_) => Err(MissingRd { intid, affinity }),",
        'Err(_) => { error!("gicv3: helper miss"); Err(MissingRd { intid, affinity }) }'),
    # wrapper captures raw MPIDR low bits instead of the packed affinity
    "wrong_affinity": ("wrapper",
        "let affinity = model::mpidr_affinity(current_mpidr());",
        "let affinity = current_mpidr() as u32;"),
    # helper records a truncated affinity in the diagnostic
    "wrong_diag_affinity": ("helper",
        "Err(_) => Err(MissingRd { intid, affinity }),",
        "Err(_) => Err(MissingRd { intid, affinity: affinity & 0x00ff_ffff }),"),
    # wrapper's legacy failure log is suppressed
    "suppressed_log": ("wrapper", None, None),
}


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def extract(lines, sig_re, what):
    """Return (start, end) 0-based inclusive: leading ///, #[..] lines + braced body."""
    hits = [i for i, l in enumerate(lines) if re.search(sig_re, l)]
    if len(hits) != 1:
        sys.exit(f"extract {what}: expected 1 signature match, got {hits}")
    start = hits[0]
    while start > 0 and lines[start - 1].lstrip().startswith(("///", "#[")):
        start -= 1
    depth, seen = 0, False
    for end in range(hits[0], len(lines)):
        depth += lines[end].count("{") - lines[end].count("}")
        seen = seen or "{" in lines[end]
        if seen and depth == 0:
            return start, end
    sys.exit(f"extract {what}: unbalanced braces")


def block(lines, rng):
    return "\n".join(lines[rng[0]:rng[1] + 1]) + "\n"


def mutate(text, mutant):
    if mutant == "suppressed_log":
        new, n = re.subn(r"\n\s*error!\(\n(?:.*\n)*?\s*\);", "", text, count=1)
    else:
        old, rep = MUTANTS[mutant][1], MUTANTS[mutant][2]
        n = text.count(old)
        new = text.replace(old, rep)
    if n != 1 or new == text:
        sys.exit(f"mutant {mutant}: anchor matched {n} times")
    return new


def static_checks(helper, diag):
    code = lambda t: "\n".join(l for l in t.split("\n") if not l.lstrip().startswith("//"))
    helper, diag = code(helper), code(diag)
    macro = re.findall(r"\b\w+!\s*[\(\[\{]", helper)
    banned = [w for w in ("alloc", "Vec", "Box", "String", "format", "dyn", "Fn", "impl ", "&mut")
              if w in helper]
    return {
        "helper_macro_invocations": macro,
        "helper_banned_tokens": banned,
        "helper_has_closure_pipe": "|" in helper,
        "diag_has_reference_or_lifetime": ("&" in diag) or ("'" in diag),
        "pass": not macro and not banned and "|" not in helper
                and "&" not in diag and "'" not in diag,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mutant", default="none", choices=["none", *MUTANTS])
    args = ap.parse_args()

    cur_src = (WORKTREE / REL).read_text()
    base_src = (HERE / "before.rs").read_text()
    assert sha(base_src) == "4acca44f812ad8c295b9e06005698b6fe8062e15aa09dd113f4872e5fee1a4d5", "baseline fixture changed"
    model_cur = (WORKTREE / MODEL_REL).read_text()
    model_base = model_cur
    assert sha(model_cur) == "ef1b66da29c31d9a6550af3dcb8f398b4ad688bfe965ee8043807c3c6d46eaa1", "model source changed; review required"
    cl, bl = cur_src.split("\n"), base_src.split("\n")

    r_diag = extract(cl, r"^struct MissingRd \{", "MissingRd")
    r_help = extract(cl, r"^fn lookup_private_rd\(", "lookup_private_rd")
    r_wrap = extract(cl, r"^    fn private_rd\(&self, intid: u32\)", "current private_rd")
    r_base = extract(bl, r"^    fn private_rd\(&self, intid: u32\)", "baseline private_rd")
    diag, helper = block(cl, r_diag), block(cl, r_help)
    wrapper, base_wrapper = block(cl, r_wrap), block(bl, r_base)

    # Outside the extraction, current must equal baseline byte-for-byte.
    recon = cl[:r_wrap[0]] + bl[r_base[0]:r_base[1] + 1] + cl[r_wrap[1] + 1:]
    new_items = (r_diag[0] - 1, r_help[1])  # blank line before diag through helper end
    assert cl[new_items[0]] == "" and r_diag[1] + 1 < r_help[0] and r_help[1] < r_wrap[0]
    shift = new_items[1] - new_items[0] + 1
    recon = recon[:new_items[0]] + recon[new_items[1] + 1:]
    outside_identical = "\n".join(recon) == base_src
    if not outside_identical:
        sys.exit("code outside the extraction differs from baseline")

    if args.mutant != "none":
        part = MUTANTS[args.mutant][0]
        if part == "helper":
            helper = mutate(helper, args.mutant)
        else:
            wrapper = mutate(wrapper, args.mutant)

    prelude = ("// GENERATED by gen.py -- do not edit. mutant={m}\n"
               "#![allow(dead_code, unused_variables)]\n"
               "use crate::model::{{self, RdRegistry}};\n"
               "use crate::shim::current_mpidr;\n\n"
               "pub struct GicV3 {{\n    pub rd_registry: RdRegistry,\n}}\n\n")
    call = ("impl GicV3 {\n    pub fn call_private_rd(&self, intid: u32) -> Option<usize> {\n"
            "        self.private_rd(intid)\n    }\n}\n")
    GEN.mkdir(parents=True, exist_ok=True)
    (GEN / "baseline.rs").write_text(
        prelude.format(m="n/a (accepted26 fixture)") + "impl GicV3 {\n" + base_wrapper + "}\n\n" + call)
    (GEN / "current.rs").write_text(
        prelude.format(m=args.mutant) + diag + "\n" + helper + "\nimpl GicV3 {\n" + wrapper + "}\n\n"
        + call + '\n#[cfg(test)]\n#[path = "../current_tests.rs"]\nmod tests;\n')

    if args.mutant == "none":
        prov = {
            "base_rev": BASE_REV,
            "worktree_head": subprocess.check_output(["git", "-C", WORKTREE, "rev-parse", "HEAD"], text=True).strip(),
            "gicv3_rs_fixture_sha256": sha(base_src),
            "gicv3_rs_worktree_sha256": sha(cur_src),
            "model_rs_expected_sha256": sha(model_base),
            "model_rs_worktree_sha256": sha(model_cur),
            "model_rs_unchanged": model_base == model_cur,
            "extraction_lines_1based": {
                "MissingRd(current)": [r_diag[0] + 1, r_diag[1] + 1],
                "lookup_private_rd(current)": [r_help[0] + 1, r_help[1] + 1],
                "private_rd(current)": [r_wrap[0] + 1, r_wrap[1] + 1],
                "private_rd(fixture)": [r_base[0] + 1, r_base[1] + 1],
            },
            "snippet_sha256": {"MissingRd": sha(diag), "lookup_private_rd": sha(helper),
                               "private_rd_current": sha(wrapper), "private_rd_fixture": sha(base_wrapper)},
            "outside_extraction_identical_to_fixture": outside_identical,
            "inserted_lines": shift,
            "static_checks": static_checks(helper, diag),
        }
        (HERE.parent / "provenance.json").write_text(json.dumps(prov, indent=2) + "\n")
        if not prov["static_checks"]["pass"] or not prov["model_rs_unchanged"]:
            sys.exit(f"static/provenance check failed: {prov['static_checks']}")
    print(f"generated mutant={args.mutant}")


if __name__ == "__main__":
    main()
