//! Bounded zero-copy structural reader for the safetensors container format.
//!
//! This validates the *structure* of a safetensors file: the 8-byte little-endian
//! header length, the JSON header (tensor descriptors + optional string metadata),
//! and the tensor byte ranges inside the payload. Tensor payloads are returned as
//! borrowed `&[u8]` slices pointing *inside* the caller's input buffer — no tensor
//! contents are copied, allocated, mmapped, or numerically interpreted here.
//!
//! Format reference (independently reviewed upstream README, "Format" section):
//! 8-byte little-endian u64 header length N, then N bytes of UTF-8 JSON header,
//! then the data section. Header JSON maps tensor names to descriptors with
//! `dtype`, `shape`, and `data_offsets` (`[begin, end)`, relative to the start of
//! the data section, contiguous row-major little-endian). Optional reserved key
//! `__metadata__` maps strings to strings. Duplicate JSON keys are forbidden at
//! every level.
//!
//! Header bytes and total input size are checked before JSON allocation;
//! tensor count, rank and dimensions are checked after metadata parsing. This is a
//! *strict-resource-policy subset* of the format: a file that is structurally
//! valid but exceeds a configured limit is rejected as `LimitExceeded`, not
//! parsed. Header parsing can allocate more memory than the header byte count; this is
//! not literally zero-allocation parsing, only zero *tensor-content* allocation.
//!
//! Hash/provenance verification, quantized dtypes, and full-model tensor loading
//! are later gates and are NOT claimed here.

use std::collections::BTreeMap;
#[path = "artifact_json.rs"]
mod json;
#[path = "artifact_types.rs"]
mod types;
use json::parse_unique_object;
use types::dtype_width;
pub use types::{Artifact, ArtifactError, ArtifactLimits, TensorView, RESERVED_METADATA_KEY};

/// Parse a safetensors byte slice into a borrowed structural view.
///
/// The input must be the complete file bytes (8-byte length prefix + header +
/// payload). All returned slices borrow from `input`.
pub fn parse_artifact<'a>(
    input: &'a [u8],
    limits: &ArtifactLimits,
) -> Result<Artifact<'a>, ArtifactError> {
    // --- header length prefix --------------------------------------------
    if input.len() < 8 {
        return Err(ArtifactError::TruncatedPrefix);
    }
    let mut len_buf = [0u8; 8];
    len_buf.copy_from_slice(&input[..8]);
    let header_len = u64::from_le_bytes(len_buf);

    if header_len > limits.max_header_bytes {
        return Err(ArtifactError::HeaderTooLarge);
    }
    // Checked conversion + addition before any indexing.
    let header_end = 8u64
        .checked_add(header_len)
        .ok_or(ArtifactError::InputTooLarge)?;
    if header_end > limits.max_total_bytes {
        return Err(ArtifactError::InputTooLarge);
    }
    let header_end = usize::try_from(header_end).map_err(|_| ArtifactError::InputTooLarge)?;
    if header_end > input.len() {
        return Err(ArtifactError::TruncatedHeader);
    }

    // Total input size policy: the whole buffer must fit the caller limit.
    if (input.len() as u64) > limits.max_total_bytes {
        return Err(ArtifactError::InputTooLarge);
    }

    // --- header bytes ------------------------------------------------------
    let header_bytes = &input[8..header_end];
    if header_bytes.first() != Some(&b'{') {
        return Err(ArtifactError::HeaderNotObject);
    }
    let header_str = std::str::from_utf8(header_bytes).map_err(|_| ArtifactError::HeaderNotUtf8)?;

    let payload: &[u8] = &input[header_end..];

    // --- header JSON with duplicate-key rejection at every level ----------
    let root = parse_unique_object(header_str)?;

    let mut metadata: Option<BTreeMap<String, String>> = None;
    let mut descriptors: Vec<(String, String, Vec<u64>, [u64; 2])> = Vec::new();

    for (name, desc) in root {
        if name == RESERVED_METADATA_KEY {
            let obj = desc
                .as_object()
                .ok_or(ArtifactError::MetadataNotStringMap)?;
            let mut map = BTreeMap::new();
            for (k, v) in obj {
                let s = v.as_str().ok_or(ArtifactError::MetadataNotStringMap)?;
                map.insert(k.clone(), s.to_string());
            }
            metadata = Some(map);
            continue;
        }
        if descriptors.len() >= limits.max_tensors {
            return Err(ArtifactError::LimitExceeded);
        }
        let (dtype, shape, offsets) = parse_descriptor(&desc, limits)?;
        descriptors.push((name, dtype, shape, offsets));
    }

    // --- dtype support -----------------------------------------------------
    for (_, dtype, _, _) in &descriptors {
        if dtype_width(dtype).is_none() {
            return Err(ArtifactError::UnsupportedDtype(dtype.clone()));
        }
    }

    // --- range validation against payload ----------------------------------
    let payload_len = payload.len() as u64;
    for (_, _, _, offsets) in &descriptors {
        let [begin, end] = *offsets;
        if end < begin {
            return Err(ArtifactError::ReversedOffsets);
        }
        if end > payload_len {
            return Err(ArtifactError::OffsetOutOfRange);
        }
    }

    // Shape product must equal range length in bytes for the dtype width.
    for (_, dtype, shape, offsets) in &descriptors {
        let width = dtype_width(dtype).expect("checked above");
        let elems = if shape.contains(&0) {
            0
        } else {
            shape
                .iter()
                .try_fold(1_u64, |count, &dimension| count.checked_mul(dimension))
                .ok_or(ArtifactError::ShapeProductOverflow)?
        };
        let want = elems
            .checked_mul(width as u64)
            .ok_or(ArtifactError::ShapeProductOverflow)?;
        if offsets[1] - offsets[0] != want {
            return Err(ArtifactError::ShapeByteMismatch);
        }
    }

    // Contiguity: sort by (begin, end); ranges must tile [0, payload_len)
    // exactly. Empty ranges sharing a boundary sort by end as tie-break so
    // [0,0] precedes [0,4], avoiding a false overlap.
    descriptors.sort_by_key(|d| (d.3[0], d.3[1]));
    let mut cursor: u64 = 0;
    for (_, _, _, offsets) in &descriptors {
        if offsets[0] < cursor {
            return Err(ArtifactError::OverlappingRanges);
        }
        if offsets[0] > cursor {
            return Err(ArtifactError::RangeHole);
        }
        cursor = offsets[1];
    }
    if cursor != payload_len {
        return Err(ArtifactError::UnindexedTrailingBytes);
    }

    // --- build borrowed views ----------------------------------------------
    let mut tensors = Vec::with_capacity(descriptors.len());
    for (name, dtype, shape, offsets) in descriptors {
        let bytes = &payload[offsets[0] as usize..offsets[1] as usize];
        tensors.push(TensorView {
            name,
            dtype,
            shape,
            data_offsets: offsets,
            bytes,
        });
    }

    Ok(Artifact {
        tensors,
        metadata,
        payload,
    })
}

fn parse_descriptor(
    desc: &serde_json::Value,
    limits: &ArtifactLimits,
) -> Result<(String, Vec<u64>, [u64; 2]), ArtifactError> {
    let obj = desc.as_object().ok_or(ArtifactError::DescriptorNotObject)?;
    let mut dtype: Option<String> = None;
    let mut shape: Option<Vec<u64>> = None;
    let mut offsets: Option<[u64; 2]> = None;

    for (field, val) in obj {
        match field.as_str() {
            "dtype" => {
                if dtype.is_some() {
                    return Err(ArtifactError::DuplicateDescriptorField("dtype".into()));
                }
                dtype = Some(
                    val.as_str()
                        .ok_or(ArtifactError::DtypeNotString)?
                        .to_string(),
                );
            }
            "shape" => {
                if shape.is_some() {
                    return Err(ArtifactError::DuplicateDescriptorField("shape".into()));
                }
                let arr = val.as_array().ok_or(ArtifactError::ShapeNotArray)?;
                if arr.len() > limits.max_rank {
                    return Err(ArtifactError::RankTooLarge);
                }
                let mut dims = Vec::with_capacity(arr.len());
                for d in arr {
                    let i = d.as_u64().ok_or_else(|| {
                        ArtifactError::ShapeElementNotDimension(field_dup_name(d))
                    })?;
                    if i > limits.max_dimension {
                        return Err(ArtifactError::DimensionTooLarge);
                    }
                    dims.push(i);
                }
                shape = Some(dims);
            }
            "data_offsets" => {
                if offsets.is_some() {
                    return Err(ArtifactError::DuplicateDescriptorField(
                        "data_offsets".into(),
                    ));
                }
                let arr = val.as_array().ok_or(ArtifactError::OffsetsNotPair)?;
                if arr.len() != 2 {
                    return Err(ArtifactError::OffsetsNotPair);
                }
                let mut pair = [0u64; 2];
                for (slot, o) in pair.iter_mut().zip(arr) {
                    let i = o
                        .as_u64()
                        .ok_or_else(|| ArtifactError::OffsetNotUnsigned(field_dup_name(o)))?;
                    *slot = i;
                }
                offsets = Some(pair);
            }
            other => {
                return Err(ArtifactError::UnknownDescriptorField(other.to_string()));
            }
        }
    }

    let dtype = dtype.ok_or_else(|| ArtifactError::MissingDescriptorField("dtype".into()))?;
    let shape = shape.ok_or_else(|| ArtifactError::MissingDescriptorField("shape".into()))?;
    let offsets =
        offsets.ok_or_else(|| ArtifactError::MissingDescriptorField("data_offsets".into()))?;
    Ok((dtype, shape, offsets))
}

fn field_dup_name(value: &serde_json::Value) -> String {
    value.to_string()
}
