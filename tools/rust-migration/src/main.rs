use regex::Regex;
use serde::Serialize;
use std::{fs, path::PathBuf, process::ExitCode};

mod values;

struct Args {
    raw: PathBuf,
    jsonl: PathBuf,
    virtual_base: u128,
    physical_base: u128,
    size: u128,
    caller: Option<String>,
    dry_run: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut virtual_base = None;
    let mut physical_base = None;
    let mut size = None;
    let mut caller = None;
    let mut dry_run = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("Convert mapped Linux rwmmio events to mmio-replay physical-address JSONL.\n\nUSAGE: redox-trace-convert <raw> <jsonl> --virtual-base <N> --physical-base <N> --size <N> [--caller <REGEX>] [--dry-run]");
                std::process::exit(0);
            }
            "--virtual-base" => {
                virtual_base = Some(parse_int(
                    &args.next().ok_or("--virtual-base requires a value")?,
                )?)
            }
            "--physical-base" => {
                physical_base = Some(parse_int(
                    &args.next().ok_or("--physical-base requires a value")?,
                )?)
            }
            "--size" => size = Some(parse_int(&args.next().ok_or("--size requires a value")?)?),
            "--caller" => caller = Some(args.next().ok_or("--caller requires a value")?),
            "--dry-run" => dry_run = true,
            v if v.starts_with('-') => return Err(format!("unknown argument: {v}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if positional.len() != 2 {
        return Err("expected raw and JSONL paths".into());
    }
    Ok(Args {
        raw: positional.remove(0),
        jsonl: positional.remove(0),
        virtual_base: virtual_base.ok_or("--virtual-base is required")?,
        physical_base: physical_base.ok_or("--physical-base is required")?,
        size: size.ok_or("--size is required")?,
        caller,
        dry_run,
    })
}

#[derive(Debug, PartialEq, Serialize)]
struct Record {
    op: &'static str,
    width: u32,
    addr: String,
    value: String,
    delay_us: u128,
}

/// Python `int(s, 0)` parity for positive ASCII literals only. Arbitrary
/// precision, signed negatives, and Unicode digits remain explicit gaps.
fn parse_int(s: &str) -> Result<u128, String> {
    let invalid = || Err(format!("invalid literal for int() with base 0: '{s}'"));
    let rest = s.strip_prefix('+').unwrap_or(s);
    let (digits, radix, prefixed) = match rest.as_bytes() {
        [b'0', b'x' | b'X', tail @ ..] => (std::str::from_utf8(tail).unwrap_or(""), 16, true),
        [b'0', b'o' | b'O', tail @ ..] => (std::str::from_utf8(tail).unwrap_or(""), 8, true),
        [b'0', b'b' | b'B', tail @ ..] => (std::str::from_utf8(tail).unwrap_or(""), 2, true),
        _ => (rest, 10, false),
    };
    // One underscore is permitted directly after an explicit radix prefix only.
    let digits = if prefixed {
        digits.strip_prefix('_').unwrap_or(digits)
    } else {
        digits
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return invalid();
    }
    // Decimal leading zero is legal only when every digit is zero (separators ignored).
    if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
        let all_zero = digits.bytes().all(|b| b == b'0' || b == b'_');
        if !all_zero {
            return invalid();
        }
    }
    let mut value: u128 = 0;
    let mut prev_digit = false;
    for byte in digits.bytes() {
        if byte == b'_' {
            if !prev_digit {
                return invalid();
            }
            prev_digit = false;
            continue;
        }
        let Some(d) = (byte as char).to_digit(radix) else {
            return invalid();
        };
        value = match value
            .checked_mul(u128::from(radix))
            .and_then(|v| v.checked_add(u128::from(d)))
        {
            Some(v) => v,
            None => return Err(format!("integer out of u128 range: '{s}'")),
        };
        prev_digit = true;
    }
    Ok(value)
}

fn timestamp_ns(line: &str, re: &Regex) -> Option<u128> {
    let caps = re.captures(line)?;
    let whole: u128 = caps.get(1)?.as_str().parse().ok()?;
    let frac = caps.get(2)?.as_str();
    let frac9 = format!("{frac:0<9}");
    let frac_ns: u128 = frac9[..9].parse().ok()?;
    whole.checked_mul(1_000_000_000)?.checked_add(frac_ns)
}

fn convert(
    lines: &str,
    virt: u128,
    phys: u128,
    size: u128,
    caller: Option<&Regex>,
) -> Result<Vec<Record>, String> {
    let event_re = Regex::new(r"\brwmmio_(read|post_read|write|post_write):\s*(.*)").unwrap();
    let field_re = Regex::new(r"\b(caller|width|val|addr)=([^\s]+)").unwrap();
    let stamp_re = Regex::new(r"\s(\d+)\.(\d+):\s+rwmmio_").unwrap();
    let mut output = Vec::new();
    let mut previous_ns: Option<u128> = None;
    let end = virt.checked_add(size).ok_or("mapping range overflows")?;

    for (idx, line) in lines.lines().enumerate() {
        let Some(ev) = event_re.captures(line) else {
            continue;
        };
        let kind = ev.get(1).unwrap().as_str();
        if kind != "post_read" && kind != "write" {
            continue;
        }
        let body = ev.get(2).unwrap().as_str();
        let fields: std::collections::HashMap<&str, &str> = field_re
            .captures_iter(body)
            .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str()))
            .collect();
        let line_no = idx + 1;
        let (Some(width_s), Some(value_s), Some(addr_s)) =
            (fields.get("width"), fields.get("val"), fields.get("addr"))
        else {
            return Err(format!("line {line_no}: incomplete rwmmio event"));
        };
        if let Some(re) = caller {
            if !re.is_match(fields.get("caller").copied().unwrap_or("")) {
                continue;
            }
        }
        let width_raw =
            parse_int(width_s).map_err(|e| format!("line {line_no}: invalid field: {e}"))?;
        let width = match width_raw {
            1 => 8,
            2 => 16,
            4 => 32,
            8 => 64,
            n => u32::try_from(n).unwrap_or(0),
        };
        let addr = parse_int(addr_s).map_err(|e| format!("line {line_no}: invalid field: {e}"))?;
        if ![8, 16, 32, 64].contains(&width) {
            return Err(format!("line {line_no}: unsupported width {width_raw}"));
        }
        let value = values::masked_value(value_s, width)
            .map_err(|e| format!("line {line_no}: invalid field: {e}"))?;
        let bytes = u128::from(width / 8);
        if addr < virt || addr >= end || addr.checked_add(bytes).is_none_or(|last| last > end) {
            continue;
        }
        let now = timestamp_ns(line, &stamp_re);
        let delay = match (now, previous_ns) {
            (Some(n), Some(p)) if n > p => (n - p) / 1_000,
            _ => 0,
        };
        previous_ns = now;
        let physical = phys
            .checked_add(addr - virt)
            .ok_or("mapped physical address overflows")?;
        let mask = (1u128 << width) - 1;
        output.push(Record {
            op: if kind == "post_read" { "r" } else { "w" },
            width,
            addr: format!("0x{physical:x}"),
            value: format!("0x{:x}", value & mask),
            delay_us: delay,
        });
    }
    Ok(output)
}

fn resolved(path: &PathBuf) -> std::io::Result<PathBuf> {
    if path.exists() {
        fs::canonicalize(path)
    } else {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        Ok(fs::canonicalize(parent)?.join(path.file_name().unwrap_or_default()))
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if args.size == 0 {
        return Err("mapping bases must be nonnegative and size positive".into());
    }
    if resolved(&args.raw).map_err(|e| e.to_string())?
        == resolved(&args.jsonl).map_err(|e| e.to_string())?
    {
        return Err("raw and JSONL paths must differ".into());
    }
    let caller = args
        .caller
        .as_deref()
        .map(Regex::new)
        .transpose()
        .map_err(|e| format!("invalid caller regex: {e}"))?;
    let raw = fs::read(&args.raw).map_err(|e| format!("{}: {e}", args.raw.display()))?;
    let raw = String::from_utf8_lossy(&raw);
    let records = convert(
        &raw,
        args.virtual_base,
        args.physical_base,
        args.size,
        caller.as_ref(),
    )?;
    if records.is_empty() {
        return Err(
            "no mapped rwmmio events; check tracepoint availability and VA-to-PA mapping".into(),
        );
    }
    if !args.dry_run {
        let mut bytes = Vec::new();
        for rec in &records {
            serde_json::to_writer(&mut bytes, rec).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
        }
        fs::write(&args.jsonl, bytes).map_err(|e| format!("{}: {e}", args.jsonl.display()))?;
    }
    eprintln!(
        "{} MMIO accesses -> {}{}",
        records.len(),
        args.jsonl.display(),
        if args.dry_run { " (dry run)" } else { "" }
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_unique_accesses_and_delays() {
        let raw = "task [000] 1.000002: rwmmio_post_read: caller=geni width=4 val=0xab addr=0xffff1004\ntask [000] 1.000009: rwmmio_write: caller=geni width=4 val=0x12345678 addr=0xffff1008\n";
        let records = convert(raw, 0xffff1000, 0xa84000, 0x1000, None).unwrap();
        assert_eq!(
            records,
            vec![
                Record {
                    op: "r",
                    width: 32,
                    addr: "0xa84004".into(),
                    value: "0xab".into(),
                    delay_us: 0
                },
                Record {
                    op: "w",
                    width: 32,
                    addr: "0xa84008".into(),
                    value: "0x12345678".into(),
                    delay_us: 7
                }
            ]
        );
    }

    #[test]
    fn rejects_incomplete_and_unsupported_widths() {
        assert!(
            convert("x: rwmmio_write: width=4 val=0x1", 0, 0, 0x100, None)
                .unwrap_err()
                .contains("incomplete")
        );
        assert!(
            convert("x: rwmmio_write: width=3 val=0x1 addr=0", 0, 0, 0x100, None)
                .unwrap_err()
                .contains("unsupported width")
        );
    }

    #[test]
    fn filters_callers_and_mapping_window_and_masks_values() {
        let raw = "x 1.0: rwmmio_write: caller=other width=1 val=0x1ff addr=0x1000\nx 1.0: rwmmio_write: caller=geni width=1 val=0x1ff addr=0x1001\nx 1.0: rwmmio_write: caller=geni width=1 val=0x1 addr=0x2000\n";
        let re = Regex::new("geni").unwrap();
        let result = convert(raw, 0x1000, 0x4000, 0x100, Some(&re)).unwrap();
        assert_eq!(result[0].value, "0xff");
        assert_eq!(result[0].addr, "0x4001");
        assert_eq!(result.len(), 1);
    }
}
