//! Integration tests for the bounded safetensors structural reader.
//! Tests build real safetensors byte buffers and assert structural acceptance
//! or precise rejection — they do not echo implementation internals.

use lfm3_brain::artifacts::{parse_artifact, ArtifactLimits};

/// Build a complete safetensors file: 8-byte LE header length + header JSON + payload.
fn st(header_json: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(header_json.len() as u64).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(payload);
    out
}

fn limits() -> ArtifactLimits {
    ArtifactLimits {
        max_header_bytes: 4096,
        max_tensors: 16,
        max_rank: 4,
        max_dimension: u64::MAX,
        max_total_bytes: 1 << 20,
    }
}

fn f32s(vals: &[f32]) -> Vec<u8> {
    let mut v = Vec::new();
    for x in vals {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

#[test]
fn accepts_distinct_supported_dtypes() {
    // One tensor per dtype; each range is exactly 2 * dtype width bytes.
    let dtypes: [(&str, usize); 13] = [
        ("F64", 8),
        ("F32", 4),
        ("F16", 2),
        ("BF16", 2),
        ("I64", 8),
        ("U64", 8),
        ("I32", 4),
        ("U32", 4),
        ("I16", 2),
        ("U16", 2),
        ("I8", 1),
        ("U8", 1),
        ("BOOL", 1),
    ];
    let mut header = String::from("{");
    let mut payload = Vec::new();
    let mut off = 0u64;
    for (i, (d, w)) in dtypes.iter().enumerate() {
        if i > 0 {
            header.push(',');
        }
        let len = (2 * w) as u64;
        header.push_str(&format!(
            "\"t{i}\":{{\"dtype\":\"{d}\",\"shape\":[2],\"data_offsets\":[{off},{}]}}",
            off + len
        ));
        payload.extend_from_slice(&vec![0xABu8; len as usize]);
        off += len;
    }
    header.push('}');
    let file = st(&header, &payload);
    let art = parse_artifact(&file, &limits()).expect("all byte-aligned dtypes accepted");
    assert_eq!(art.tensors.len(), dtypes.len());
    for (i, (d, w)) in dtypes.iter().enumerate() {
        let t = art.tensor(&format!("t{i}")).expect("tensor present");
        assert_eq!(t.dtype, *d);
        assert_eq!(t.bytes.len(), 2 * w);
    }
}

#[test]
fn accepts_rank0_scalar_and_empty_tensors_sharing_boundary() {
    // scalar (rank 0) F32 = 4 bytes at [0,4), two empty tensors sharing
    // boundary 4, then a 2-element F32 tensor at [4,12). Payload = 12 bytes.
    let header = r#"{"s":{"dtype":"F32","shape":[],"data_offsets":[0,4]},
        "e1":{"dtype":"BF16","shape":[0],"data_offsets":[4,4]},
        "e2":{"dtype":"U8","shape":[3,0],"data_offsets":[4,4]},
        "v":{"dtype":"F32","shape":[2],"data_offsets":[4,12]}}"#;
    let payload = {
        // [0,4) scalar, [4,12) vector: 12 payload bytes total.
        let mut p = f32s(&[1.5, -2.25]);
        p.extend_from_slice(&0.0f32.to_le_bytes()); // filler inside v's range
        p
    };
    assert_eq!(payload.len(), 12);
    let file = st(header, &payload);
    let art = parse_artifact(&file, &limits()).expect("scalar + shared-boundary empties valid");
    let s = art.tensor("s").unwrap();
    assert_eq!(s.shape.len(), 0);
    assert_eq!(s.element_count(), 1); // empty product = 1
    assert_eq!(s.bytes, &payload[0..4]); // 1.5f32 LE
    assert_eq!(art.tensor("e1").unwrap().bytes.len(), 0);
    assert_eq!(art.tensor("e2").unwrap().bytes.len(), 0);
    assert_eq!(art.tensor("v").unwrap().element_count(), 2);
    assert_eq!(art.tensor("v").unwrap().bytes, &payload[4..12]);
}

#[test]
fn accepts_string_metadata_and_borrows_payload_from_input() {
    let header = r#"{"w":{"dtype":"I32","shape":[2],"data_offsets":[0,8]},"__metadata__":{"format":"pt","author":"x"}}"#;
    let payload = vec![1u8, 0, 0, 0, 2, 0, 0, 0];
    let file = st(header, &payload);
    let art = parse_artifact(&file, &limits()).expect("valid with metadata");
    let md = art.metadata.as_ref().expect("metadata present");
    assert_eq!(md.get("format").map(String::as_str), Some("pt"));
    assert_eq!(md.get("author").map(String::as_str), Some("x"));

    // Borrowed payload must point strictly inside the original input buffer.
    let t = art.tensor("w").unwrap();
    let in_start = file.as_ptr() as usize;
    let in_end = in_start + file.len();
    let p_start = t.bytes.as_ptr() as usize;
    assert!(p_start > in_start && p_start + t.bytes.len() <= in_end);
    assert_eq!(t.bytes, &payload[..]);
}
