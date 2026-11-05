//! Safetensors structural-view types and resource policy.
use std::collections::BTreeMap;
/// Explicit caller-supplied resource limits for one parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactLimits {
    /// Maximum accepted header length (the u64 after the first 8 bytes).
    pub max_header_bytes: u64,
    /// Maximum number of tensor descriptors accepted.
    pub max_tensors: usize,
    /// Maximum tensor rank (number of shape dimensions) accepted.
    pub max_rank: usize,
    /// Maximum accepted value of any individual dimension.
    pub max_dimension: u64,
    /// Maximum accepted total input size (header prefix + header + payload).
    pub max_total_bytes: u64,
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        Self {
            // Header byte policy, not a heap-allocation byte limit.
            max_header_bytes: 1 << 20,
            max_tensors: 4096,
            max_rank: 8,
            max_dimension: u64::MAX,
            // 64 MiB total: strict subset policy; caller may raise it.
            max_total_bytes: 64 << 20,
        }
    }
}

/// Structural parse failure. Each variant names the violated invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    /// Input shorter than the 8-byte header-length prefix.
    TruncatedPrefix,
    /// Header length exceeds `max_header_bytes`.
    HeaderTooLarge,
    /// 8 + header length overflows u64 or exceeds `max_total_bytes`.
    InputTooLarge,
    /// Input shorter than the declared header end.
    TruncatedHeader,
    /// Header bytes are not valid UTF-8.
    HeaderNotUtf8,
    /// Header does not start with `{`.
    HeaderNotObject,
    /// Header JSON is malformed or truncated.
    HeaderJson(String),
    /// A root key in the header object is not a string.
    RootKeyNotString,
    /// A tensor name appears more than once at the header root.
    DuplicateTensorName(String),
    /// A tensor descriptor is not a JSON object.
    DescriptorNotObject,
    /// A descriptor field name is not one of dtype/shape/data_offsets.
    UnknownDescriptorField(String),
    /// A descriptor field appears more than once.
    DuplicateDescriptorField(String),
    /// A descriptor is missing one of dtype/shape/data_offsets.
    MissingDescriptorField(String),
    /// `dtype` is not a string.
    DtypeNotString,
    /// `shape` is not an array of integers.
    ShapeNotArray,
    /// A shape element is negative or not an integer.
    ShapeElementNotDimension(String),
    /// Shape rank exceeds `max_rank`.
    RankTooLarge,
    /// A dimension exceeds the configured policy limit.
    DimensionTooLarge,
    /// The product of shape dimensions overflows u64.
    ShapeProductOverflow,
    /// `data_offsets` is not a 2-element array of integers.
    OffsetsNotPair,
    /// An offset element is negative or not an integer.
    OffsetNotUnsigned(String),
    /// `__metadata__` is not an object of string to string.
    MetadataNotStringMap,
    /// A metadata key appears more than once.
    DuplicateMetadataKey(String),
    /// The dtype is not in the supported byte-aligned scalar set (including
    /// quantized dtypes); its payload is never treated as plain bytes.
    UnsupportedDtype(String),
    /// `data_offsets` are reversed (end < begin).
    ReversedOffsets,
    /// An offset range exceeds the actual payload size.
    OffsetOutOfRange,
    /// dtype element size times shape product does not equal the range length.
    ShapeByteMismatch,
    /// Tensor byte ranges overlap.
    OverlappingRanges,
    /// Tensor byte ranges leave a hole (not starting at 0 or a gap between ranges).
    RangeHole,
    /// Payload bytes after the last tensor range are not referenced by any tensor.
    UnindexedTrailingBytes,
    /// A caller limit (tensor count) was exceeded.
    LimitExceeded,
}

/// The reserved header key holding string→string metadata.
pub const RESERVED_METADATA_KEY: &str = "__metadata__";

/// Byte width of a supported scalar dtype. Unsupported and quantized dtypes
/// are rejected before this is consulted; they are never treated as byte blobs.
pub(super) fn dtype_width(dtype: &str) -> Option<usize> {
    match dtype {
        "F64" | "I64" | "U64" => Some(8),
        "F32" | "I32" | "U32" => Some(4),
        "F16" | "BF16" | "I16" | "U16" => Some(2),
        "I8" | "U8" | "BOOL" => Some(1),
        _ => None,
    }
}

/// A validated tensor description with a borrowed payload view.
#[derive(Debug)]
pub struct TensorView<'a> {
    /// Tensor name from the header root.
    pub name: String,
    /// dtype string (one of the supported scalar dtypes).
    pub dtype: String,
    /// Shape dimensions (row-major outer-to-inner).
    pub shape: Vec<u64>,
    /// `[begin, end)` byte range within the data section.
    pub data_offsets: [u64; 2],
    /// Borrowed payload bytes for this tensor — points inside the original input.
    pub bytes: &'a [u8],
}

impl<'a> TensorView<'a> {
    /// Number of elements implied by the shape (rank-0 product is 1).
    pub fn element_count(&self) -> u64 {
        if self.shape.contains(&0) {
            0
        } else {
            self.shape.iter().product()
        }
    }
}

/// A structurally validated safetensors artifact view.
#[derive(Debug)]
pub struct Artifact<'a> {
    /// Tensor views in ascending `(data_offsets.0, data_offsets.1)` order.
    pub tensors: Vec<TensorView<'a>>,
    /// Optional `__metadata__` map (string to string).
    pub metadata: Option<BTreeMap<String, String>>,
    /// The payload slice (data section) borrowed from the input.
    pub payload: &'a [u8],
}

impl<'a> Artifact<'a> {
    /// Look up a tensor view by name (linear scan; the vec is offset-ordered).
    pub fn tensor(&self, name: &str) -> Option<&TensorView<'a>> {
        self.tensors.iter().find(|t| t.name == name)
    }
}
