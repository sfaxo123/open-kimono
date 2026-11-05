//! Parse ASCII trace values modulo the MMIO width without an integer-size cap.
//! Python's broader Unicode integer grammar remains a separate parity gap.

pub fn masked_value(text: &str, width: u32) -> Result<u128, String> {
    if ![8, 16, 32, 64].contains(&width) {
        return Err("unsupported value width".into());
    }
    let (negative, unsigned) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let (radix, digits, prefixed) = match unsigned.get(..2) {
        Some("0x" | "0X") => (16, &unsigned[2..], true),
        Some("0o" | "0O") => (8, &unsigned[2..], true),
        Some("0b" | "0B") => (2, &unsigned[2..], true),
        _ => (10, unsigned, false),
    };
    // Python int(..., 0) allows one underscore immediately after a prefix.
    let digits = if prefixed {
        digits.strip_prefix('_').unwrap_or(digits)
    } else {
        digits
    };
    if digits.is_empty() {
        return Err("value has no digits".into());
    }
    let mask = (1u128 << width) - 1;
    let mut value = 0u128;
    let mut previous_digit = false;
    let mut nonzero = false;
    for byte in digits.bytes() {
        if byte == b'_' && previous_digit {
            previous_digit = false;
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a' + 10),
            b'A'..=b'F' => u32::from(byte - b'A' + 10),
            _ => return Err("invalid value digit or separator".into()),
        };
        if digit >= radix {
            return Err("value digit exceeds radix".into());
        }
        nonzero |= digit != 0;
        // width <= 64 and radix <= 16: this intermediate fits in u128.
        value = (value * u128::from(radix) + u128::from(digit)) & mask;
        previous_digit = true;
    }
    if !previous_digit || (!prefixed && digits.starts_with('0') && nonzero) {
        return Err("invalid separator or leading-zero decimal value".into());
    }
    Ok(if negative {
        0u128.wrapping_sub(value) & mask
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::masked_value;

    #[test]
    fn handles_signed_bases_and_values_larger_than_u128() {
        for (text, width, expected) in [
            ("-1", 8, 255),
            ("-0x1f", 64, 0xffff_ffff_ffff_ffe1),
            ("-0b1", 16, 65535),
            ("+0o777", 8, 255),
            ("0x_1", 8, 1),
            ("1_0", 8, 10),
            ("-00", 8, 0),
            ("-0xffffffffffffffffffffffffffffffffffff", 64, 1),
            (
                "1111111111111111111111111111111111111111111111111111111111111111111111",
                64,
                0x71c7_1c71_c71c_71c7,
            ),
        ] {
            assert_eq!(masked_value(text, width), Ok(expected), "{text}");
        }
    }

    #[test]
    fn rejects_malformed_ascii_values_and_invalid_widths() {
        for text in [
            "", "-", "+", "0x", "-0x", "0x-1", "010", "0_1", "1__0", "_1", "1_", "0x__1", "--1",
            "0_x1", "1e3", "0b2", "0o8",
        ] {
            assert!(masked_value(text, 8).is_err(), "{text}");
        }
        assert!(masked_value("1", 0).is_err());
        assert!(masked_value("1", 128).is_err());
    }
}
