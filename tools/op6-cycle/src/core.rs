//! Pure logic for the op6-cycle port: JSON escaping, allowlist parsing,
//! ledger-head rules, receipt shaping, and the FASTBOOT landing detector.
//! Everything here is side-effect free and unit-tested; main.rs mirrors
//! the bash tool's control flow around it.

/// Mirror of the bash `esc()`: backslash first, then double-quote. (The
/// bash tool does not escape control characters; neither does this.)
pub fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One allowlist line: `<sha256> <basename> <ttl_seconds>
/// <gate_evidence_ref> [ATTENDED-ONLY]`. Mirrors the bash `read -r a_sha
/// a_base a_ttl a_gate a_mode _rest` split (whitespace fields; fields
/// past the fifth are ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowEntry {
    pub sha: String,
    pub base: String,
    pub ttl_raw: String,
    pub gate: String,
    pub mode: String,
}

pub fn parse_allowlist(text: &str) -> Vec<AllowEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() {
            continue; // blank line: bash read leaves a_sha empty -> skip
        }
        if f[0].starts_with('#') {
            continue; // comment
        }
        let get = |i: usize| f.get(i).copied().unwrap_or("").to_string();
        out.push(AllowEntry {
            sha: get(0),
            base: get(1),
            ttl_raw: get(2),
            gate: get(3),
            mode: get(4),
        });
    }
    out
}

/// Bash loop semantics: every matching line overwrites ENTRY, so the
/// LAST line matching both sha and basename wins.
pub fn find_entry<'a>(entries: &'a [AllowEntry], sha: &str, base: &str) -> Option<&'a AllowEntry> {
    entries
        .iter()
        .filter(|e| e.sha == sha && e.base == base)
        .last()
}

/// The bash TTL gate: `[ -n "$ENTRY_TTL" ] && [ "$ENTRY_TTL" -gt 0 ]`.
/// A non-numeric or non-positive TTL fails the gate (refusal).
pub fn ttl_value(raw: &str) -> Option<i64> {
    if raw.is_empty() {
        return None;
    }
    match raw.parse::<i64>() {
        Ok(v) if v > 0 => Some(v),
        _ => None,
    }
}

/// First `"outcome"` value in a receipt, mirroring
/// `grep -o '"outcome": *"[^"]*"' | head -1` + the sed extraction.
pub fn extract_outcome(text: &str) -> String {
    let pat = "\"outcome\":";
    let start = match text.find(pat) {
        Some(i) => i + pat.len(),
        None => return String::new(),
    };
    let rest = text[start..].trim_start_matches(' ');
    if !rest.starts_with('"') {
        return String::new();
    }
    let inner = &rest[1..];
    match inner.find('"') {
        Some(end) => inner[..end].to_string(),
        None => String::new(),
    }
}

/// Stop-on-first-non-PASS ledger rule: the newest receipt is a clean
/// head iff it is PASS, or FASTBOOT whose closeout verifiably returned
/// the phone to pmOS (`"recovered_to_pmos":true` in the receipt).
pub fn ledger_clean(outcome: &str, receipt_text: &str) -> bool {
    outcome == "PASS"
        || (outcome == "FASTBOOT" && receipt_text.contains("\"recovered_to_pmos\":true"))
}

/// Fields for the cycle receipt, in the bash write_receipt's exact
/// field order and formatting (single-line JSON).
pub struct ReceiptFields {
    pub started: String,
    pub finished: String,
    pub host: String,
    pub image: String,
    pub image_sha256: String,
    pub image_bytes: u64,
    pub ttl_raw: String,
    pub gate_evidence: String,
    /// `None` = unattended (JSON null); `Some(note)` = attended.
    pub attended_by: Option<String>,
    pub battery_digits: String,
    pub fastboot_serial: String,
    pub getvar_product: String,
    pub boot_sent_at: String,
    pub outcome: String,
    pub detail: String,
    pub pmos_returned_at: String,
    pub log: String,
}

/// `wc_fields` and `rec_fields` are the bash $wc_fields / $REC_FIELDS
/// fragments: either empty or starting with a comma.
pub fn receipt_json(f: &ReceiptFields, wc_fields: &str, rec_fields: &str) -> String {
    let attended_json = match &f.attended_by {
        Some(note) => format!("\"{}\"", esc(note)),
        None => "null".to_string(),
    };
    format!(
        "{{\"tool\":\"op6-cycle\",\"version\":1,\"started\":\"{started}\",\"finished\":\"{finished}\",\"host\":\"{host}\",\"image\":\"{image}\",\"image_sha256\":\"{sha}\",\"image_bytes\":{bytes},\"ttl_seconds\":{ttl},\"gate_evidence\":\"{gate}\",\"attended_by\":{attended},\"battery_pct_before\":{battery},\"fastboot_serial\":\"{serial}\",\"getvar_product\":\"{product}\",\"boot_sent_at\":\"{boot_at}\",\"outcome\":\"{outcome}\",\"detail\":\"{detail}\",\"pmos_returned_at\":\"{pmos_at}\",\"log\":\"{log}\"{wc}{rec}}}",
        started = f.started,
        finished = f.finished,
        host = esc(&f.host),
        image = esc(&f.image),
        sha = f.image_sha256,
        bytes = f.image_bytes,
        ttl = f.ttl_raw,
        gate = esc(&f.gate_evidence),
        attended = attended_json,
        battery = f.battery_digits,
        serial = f.fastboot_serial,
        product = esc(&f.getvar_product),
        boot_at = f.boot_sent_at,
        outcome = f.outcome,
        detail = esc(&f.detail),
        pmos_at = f.pmos_returned_at,
        log = esc(&f.log),
        wc = wc_fields,
        rec = rec_fields,
    )
}

/// The bash refuse() receipt: one line in receipts/refusals/.
pub fn refusal_json(reason: &str, image: &str, at: &str) -> String {
    format!(
        "{{\"tool\":\"op6-cycle\",\"outcome\":\"REFUSED\",\"reason\":\"{}\",\"image\":\"{}\",\"at\":\"{}\"}}",
        esc(reason),
        esc(image),
        at
    )
}

/// Classification of one USB poll, mirroring the bash loop's `case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Continue,
    Pass,
    Crashdump,
    Fastboot,
}

/// FASTBOOT landing detector (bash globals FB_ARMED / FB_SINCE).
///
/// Mirror of the item-3 design: the detector ARMS only after the boot
/// is proven - a post-boot poll saw a non-d00d USB state (the phone
/// actually left the bootloader). A never-booted phone sits in d00d
/// from the start and must stay HUNG, never FASTBOOT.
#[derive(Debug, Default)]
pub struct FbDetector {
    pub armed: bool,
    pub since: i64, // epoch of the first d00d sighting in this run; 0 = none
}

impl FbDetector {
    /// Feed one polled USB state at epoch `ep`. Bash order inside the
    /// d00d arm: record FB_SINCE first, then test grace + persistence.
    pub fn on_state(
        &mut self,
        state: &str,
        ep: i64,
        boot_epoch: i64,
        ttl: i64,
        grace: i64,
        persist: i64,
    ) -> Poll {
        match state {
            "0x18d1:0xd001" => Poll::Pass,
            "0x05c6:0x900e" | "0x05c6:0x9008" => Poll::Crashdump,
            "0x18d1:0xd00d" => {
                if self.armed {
                    if self.since == 0 {
                        self.since = ep;
                    }
                    if ep >= boot_epoch + ttl + grace && ep - self.since >= persist {
                        return Poll::Fastboot;
                    }
                }
                Poll::Continue
            }
            _ => {
                // Any other state (including "no device") proves the
                // phone left the bootloader: arm, reset persistence.
                self.armed = true;
                self.since = 0;
                Poll::Continue
            }
        }
    }

    /// The bash post-loop rule: armed + last sighting d00d + persisted
    /// at least FB_PERSIST at the deadline classifies FASTBOOT.
    pub fn fastboot_at_deadline(&self, last: &str, ep: i64, persist: i64) -> bool {
        self.armed && last == "0x18d1:0xd00d" && self.since > 0 && ep - self.since >= persist
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_mirrors_sed() {
        assert_eq!(esc("plain"), "plain");
        assert_eq!(esc("a\\b"), "a\\\\b");
        assert_eq!(esc("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(esc("\\\""), "\\\\\\\"");
    }

    #[test]
    fn allowlist_parse_and_find() {
        let text = "# comment line\n\
                    \n\
                    aaaa img-a.img 60 gate:a\n\
                    bbbb img-b.img 120 gate:b ATTENDED-ONLY extra fields here\n\
                    aaaa img-a.img 90 gate:a2\n";
        let entries = parse_allowlist(text);
        assert_eq!(entries.len(), 3);
        let e = find_entry(&entries, "bbbb", "img-b.img").unwrap();
        assert_eq!(e.mode, "ATTENDED-ONLY");
        assert_eq!(e.gate, "gate:b");
        // Last match wins (bash ENTRY overwrite).
        let e = find_entry(&entries, "aaaa", "img-a.img").unwrap();
        assert_eq!(e.ttl_raw, "90");
        assert!(find_entry(&entries, "aaaa", "img-b.img").is_none());
        assert!(find_entry(&entries, "cccc", "img-a.img").is_none());
    }

    #[test]
    fn allowlist_missing_fields() {
        let entries = parse_allowlist("dddd img-d.img\n");
        let e = &entries[0];
        assert_eq!(e.ttl_raw, "");
        assert_eq!(e.gate, "");
        assert_eq!(e.mode, "");
    }

    #[test]
    fn ttl_gate() {
        assert_eq!(ttl_value("120"), Some(120));
        assert_eq!(ttl_value(""), None);
        assert_eq!(ttl_value("0"), None);
        assert_eq!(ttl_value("-5"), None);
        assert_eq!(ttl_value("abc"), None);
    }

    #[test]
    fn outcome_extraction() {
        assert_eq!(
            extract_outcome("{\"tool\":\"op6-cycle\",\"outcome\":\"PASS\"}"),
            "PASS"
        );
        assert_eq!(extract_outcome("{\"outcome\": \"CRASHDUMP\"}"), "CRASHDUMP");
        assert_eq!(extract_outcome("{\"outcome\":\"FASTBOOT\",\"x\":1}"), "FASTBOOT");
        assert_eq!(extract_outcome("no outcome here"), "");
        assert_eq!(extract_outcome("{\"outcome\":}"), "");
    }

    #[test]
    fn ledger_rule() {
        assert!(ledger_clean("PASS", "{}"));
        assert!(ledger_clean(
            "FASTBOOT",
            "{\"outcome\":\"FASTBOOT\",\"recovery\":{\"recovered_to_pmos\":true}}"
        ));
        assert!(!ledger_clean(
            "FASTBOOT",
            "{\"outcome\":\"FASTBOOT\",\"recovery\":{\"recovered_to_pmos\":false}}"
        ));
        assert!(!ledger_clean("FASTBOOT", "{\"outcome\":\"FASTBOOT\"}"));
        assert!(!ledger_clean("CRASHDUMP", "{}"));
        assert!(!ledger_clean("", "{}"));
    }

    fn sample_fields() -> ReceiptFields {
        ReceiptFields {
            started: "2026-01-01T00:00:00Z".into(),
            finished: "2026-01-01T00:01:05Z".into(),
            host: "testhost".into(),
            image: "/tmp/test.img".into(),
            image_sha256: "b0423673".into(),
            image_bytes: 16,
            ttl_raw: "120".into(),
            gate_evidence: "gate-evidence:RESULT.json#deadbeef".into(),
            attended_by: None,
            battery_digits: "100".into(),
            fastboot_serial: "0a1b2c3d".into(),
            getvar_product: "product: sdm845".into(),
            boot_sent_at: "2026-01-01T00:00:03Z".into(),
            outcome: "PASS".into(),
            detail: "pmOS USB gadget enumerated after self-reset".into(),
            pmos_returned_at: "2026-01-01T00:01:05Z".into(),
            log: "/tmp/receipts/run-20260101T000000Z.log".into(),
        }
    }

    #[test]
    fn receipt_shape_unattended() {
        let j = receipt_json(&sample_fields(), "", "");
        assert_eq!(
            j,
            "{\"tool\":\"op6-cycle\",\"version\":1,\"started\":\"2026-01-01T00:00:00Z\",\"finished\":\"2026-01-01T00:01:05Z\",\"host\":\"testhost\",\"image\":\"/tmp/test.img\",\"image_sha256\":\"b0423673\",\"image_bytes\":16,\"ttl_seconds\":120,\"gate_evidence\":\"gate-evidence:RESULT.json#deadbeef\",\"attended_by\":null,\"battery_pct_before\":100,\"fastboot_serial\":\"0a1b2c3d\",\"getvar_product\":\"product: sdm845\",\"boot_sent_at\":\"2026-01-01T00:00:03Z\",\"outcome\":\"PASS\",\"detail\":\"pmOS USB gadget enumerated after self-reset\",\"pmos_returned_at\":\"2026-01-01T00:01:05Z\",\"log\":\"/tmp/receipts/run-20260101T000000Z.log\"}"
        );
    }

    #[test]
    fn receipt_shape_attended_with_fragments() {
        let mut f = sample_fields();
        f.attended_by = Some("remote order 08:00 PT".into());
        let j = receipt_json(
            &f,
            ",\"webcam_status\":\"ok\"",
            ",\"recovery\":{\"recovered_to_pmos\":true}",
        );
        assert!(j.contains("\"attended_by\":\"remote order 08:00 PT\""));
        assert!(j.ends_with(
            "\"webcam_status\":\"ok\",\"recovery\":{\"recovered_to_pmos\":true}}"
        ));
    }

    #[test]
    fn refusal_shape() {
        assert_eq!(
            refusal_json("battery 20% below floor 30%", "/tmp/x.img", "2026-01-01T00:00:00Z"),
            "{\"tool\":\"op6-cycle\",\"outcome\":\"REFUSED\",\"reason\":\"battery 20% below floor 30%\",\"image\":\"/tmp/x.img\",\"at\":\"2026-01-01T00:00:00Z\"}"
        );
    }

    #[test]
    fn detector_never_booted_stays_hung() {
        // d00d from the first poll, no non-d00d sighting ever: the
        // detector never arms, in-loop and at the deadline.
        let mut d = FbDetector::default();
        for ep in 100..110 {
            assert_eq!(
                d.on_state("0x18d1:0xd00d", ep, 100, 2, 1, 2),
                Poll::Continue
            );
        }
        assert!(!d.armed);
        assert!(!d.fastboot_at_deadline("0x18d1:0xd00d", 200, 2));
    }

    #[test]
    fn detector_arms_then_fastboot_in_loop() {
        let mut d = FbDetector::default();
        // Boot at 100, TTL 2, grace 1, persist 2 (the suite's fb_land knobs).
        assert_eq!(d.on_state("", 100, 100, 2, 1, 2), Poll::Continue); // arms
        assert_eq!(d.on_state("0x18d1:0xd00d", 101, 100, 2, 1, 2), Poll::Continue); // since=101
        assert_eq!(d.on_state("0x18d1:0xd00d", 102, 100, 2, 1, 2), Poll::Continue); // grace unmet
        assert_eq!(d.on_state("0x18d1:0xd00d", 103, 100, 2, 1, 2), Poll::Fastboot);
    }

    #[test]
    fn detector_deadline_rule() {
        let mut d = FbDetector::default();
        d.on_state("", 100, 100, 60, 30, 5);
        d.on_state("0x18d1:0xd00d", 150, 100, 60, 30, 5); // since=150, grace unmet in loop
        assert!(d.fastboot_at_deadline("0x18d1:0xd00d", 156, 5));
        assert!(!d.fastboot_at_deadline("0x18d1:0xd00d", 154, 5));
        assert!(!d.fastboot_at_deadline("", 156, 5));
    }

    #[test]
    fn detector_non_d00d_resets_persistence() {
        let mut d = FbDetector::default();
        d.on_state("", 100, 100, 2, 1, 2);
        d.on_state("0x18d1:0xd00d", 101, 100, 2, 1, 2);
        assert_eq!(d.since, 101);
        d.on_state("", 102, 100, 2, 1, 2); // left fastboot again: re-arm
        assert_eq!(d.since, 0);
        assert_eq!(d.on_state("0x18d1:0xd00d", 103, 100, 2, 1, 2), Poll::Continue);
        assert_eq!(d.since, 103);
    }

    #[test]
    fn detector_terminal_states() {
        let mut d = FbDetector::default();
        assert_eq!(d.on_state("0x18d1:0xd001", 1, 0, 60, 30, 5), Poll::Pass);
        assert_eq!(d.on_state("0x05c6:0x900e", 1, 0, 60, 30, 5), Poll::Crashdump);
        assert_eq!(d.on_state("0x05c6:0x9008", 1, 0, 60, 30, 5), Poll::Crashdump);
    }
}
