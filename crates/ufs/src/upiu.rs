//! UTRD/PRDT (little-endian) and UPIU (big-endian) encoders/decoders (`specs/ufs.md` §4).

/// Transaction code NOP OUT.
pub const NOP_OUT: u8 = 0x00;
/// Transaction code COMMAND.
pub const COMMAND: u8 = 0x01;
/// Transaction code QUERY REQUEST.
pub const QUERY_REQUEST: u8 = 0x16;
/// Transaction code NOP IN.
pub const NOP_IN: u8 = 0x20;
/// Transaction code RESPONSE.
pub const RESPONSE: u8 = 0x21;
/// Transaction code QUERY RESPONSE.
pub const QUERY_RESPONSE: u8 = 0x36;

/// Every request UPIU this driver builds is 32 bytes (header + 20 B; no data-out segment).
pub const REQ_LEN: usize = 32;
/// Header + fixed fields of every response; the data segment follows.
pub const RESP_HDR_LEN: usize = 32;

/// COMMAND flags: read (R bit in `[7:4]` = 0x4).
pub const FLAG_READ: u8 = 0x40;

/// Query function: standard read request.
pub const QUERY_FN_READ: u8 = 0x01;
/// Query function: standard write request.
pub const QUERY_FN_WRITE: u8 = 0x81;
/// Query opcode: read descriptor.
pub const QOP_READ_DESC: u8 = 0x01;
/// Query opcode: read attribute.
pub const QOP_READ_ATTR: u8 = 0x03;
/// Query opcode: read flag.
pub const QOP_READ_FLAG: u8 = 0x05;
/// Query opcode: set flag.
pub const QOP_SET_FLAG: u8 = 0x06;

/// UTRD data direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// No data phase.
    None = 0,
    /// Device to host (reads). Host-to-device (1) is never built by this driver.
    DeviceToHost = 2,
}

/// Fields of one UTRD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utrd {
    /// Data direction.
    pub dir: Direction,
    /// UCD bus address (128-byte aligned).
    pub ucd: u64,
    /// Response UPIU offset in dwords.
    pub resp_off_dw: u16,
    /// Response UPIU length in dwords.
    pub resp_len_dw: u16,
    /// PRDT offset in dwords.
    pub prdt_off_dw: u16,
    /// PRDT entry count.
    pub prdt_len: u16,
}

/// OCS value the host presets so a completion that never happened is visible.
pub const OCS_INVALID: u8 = 0x0f;

impl Utrd {
    /// Encode: command type 1 (UFS storage), interrupt bit set, OCS preset invalid.
    pub fn encode(&self) -> [u8; 32] {
        let mut d = [0u8; 32];
        let dw0 = (1u32 << 28) | ((self.dir as u32) << 25) | (1 << 24);
        d[0..4].copy_from_slice(&dw0.to_le_bytes());
        d[8] = OCS_INVALID;
        d[16..20].copy_from_slice(&(self.ucd as u32).to_le_bytes());
        d[20..24].copy_from_slice(&((self.ucd >> 32) as u32).to_le_bytes());
        d[24..26].copy_from_slice(&self.resp_len_dw.to_le_bytes());
        d[26..28].copy_from_slice(&self.resp_off_dw.to_le_bytes());
        d[28..30].copy_from_slice(&self.prdt_len.to_le_bytes());
        d[30..32].copy_from_slice(&self.prdt_off_dw.to_le_bytes());
        d
    }
}

/// One PRDT entry: `{addr lo, addr hi, reserved, byte_count - 1}`.
pub fn prdt_entry(addr: u64, len: usize) -> [u8; 16] {
    let mut e = [0u8; 16];
    e[0..4].copy_from_slice(&(addr as u32).to_le_bytes());
    e[4..8].copy_from_slice(&((addr >> 32) as u32).to_le_bytes());
    e[12..16].copy_from_slice(&((len as u32) - 1).to_le_bytes());
    e
}

/// NOP OUT UPIU.
pub fn nop_out(tag: u8) -> [u8; REQ_LEN] {
    let mut u = [0u8; REQ_LEN];
    u[0] = NOP_OUT;
    u[3] = tag;
    u
}

/// QUERY REQUEST UPIU (no data-out segment; only read/set-flag opcodes are built).
pub fn query(tag: u8, function: u8, opcode: u8, idn: u8, index: u8, length: u16) -> [u8; REQ_LEN] {
    let mut u = [0u8; REQ_LEN];
    u[0] = QUERY_REQUEST;
    u[3] = tag;
    u[5] = function;
    u[12] = opcode;
    u[13] = idn;
    u[14] = index;
    u[18..20].copy_from_slice(&length.to_be_bytes());
    u
}

/// COMMAND UPIU carrying a SCSI CDB (reads only).
pub fn command(tag: u8, lun: u8, expected_len: u32, cdb: &[u8; 16]) -> [u8; REQ_LEN] {
    let mut u = [0u8; REQ_LEN];
    u[0] = COMMAND;
    u[1] = if expected_len > 0 { FLAG_READ } else { 0 };
    u[2] = lun;
    u[3] = tag;
    u[12..16].copy_from_slice(&expected_len.to_be_bytes());
    u[16..32].copy_from_slice(cdb);
    u
}

/// The decoded fixed part of a response UPIU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RespHeader {
    /// Transaction code (low 6 bits).
    pub trans: u8,
    /// Task tag echoed by the device.
    pub tag: u8,
    /// UPIU response field (0 = target success; query response code for QUERY RESPONSE).
    pub response: u8,
    /// SCSI status (COMMAND RESPONSE).
    pub status: u8,
    /// Data segment length.
    pub data_len: u16,
}

/// Decode the response header from the first 12 bytes.
pub fn parse_header(r: &[u8]) -> RespHeader {
    RespHeader {
        trans: r[0] & 0x3f,
        tag: r[3],
        response: r[6],
        status: r[7],
        data_len: u16::from_be_bytes([r[10], r[11]]),
    }
}

/// QUERY RESPONSE value field (bytes 20-23, big-endian; flag value in byte 23).
pub fn query_value(r: &[u8]) -> u32 {
    u32::from_be_bytes([r[20], r[21], r[22], r[23]])
}

/// COMMAND RESPONSE sense bytes (length @32, data @34), clamped to the buffer.
pub fn sense(r: &[u8]) -> &[u8] {
    let n = u16::from_be_bytes([r[32], r[33]]) as usize;
    let end = (34 + n).min(r.len());
    &r[34..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utrd_layout_matches_ufshci() {
        let d = Utrd {
            dir: Direction::DeviceToHost,
            ucd: 0x1_2345_6780,
            resp_off_dw: 128,
            resp_len_dw: 128,
            prdt_off_dw: 256,
            prdt_len: 1,
        }
        .encode();
        // DW0: command type 1 [31:28], direction 2 [26:25], interrupt [24].
        assert_eq!(u32::from_le_bytes([d[0], d[1], d[2], d[3]]), 0x1500_0000);
        assert_eq!(d[8], OCS_INVALID);
        assert_eq!(
            u32::from_le_bytes([d[16], d[17], d[18], d[19]]),
            0x2345_6780
        );
        assert_eq!(u32::from_le_bytes([d[20], d[21], d[22], d[23]]), 0x1);
        assert_eq!(&d[24..32], &[128, 0, 128, 0, 1, 0, 0, 1]);
    }

    #[test]
    fn no_data_utrd_has_direction_zero() {
        let d = Utrd {
            dir: Direction::None,
            ucd: 0,
            resp_off_dw: 0,
            resp_len_dw: 0,
            prdt_off_dw: 0,
            prdt_len: 0,
        }
        .encode();
        assert_eq!(u32::from_le_bytes([d[0], d[1], d[2], d[3]]), 0x1100_0000);
    }

    #[test]
    fn prdt_byte_count_is_minus_one() {
        let e = prdt_entry(0x9_0000_1000, 4096);
        assert_eq!(&e[0..8], &[0x00, 0x10, 0, 0, 9, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes([e[12], e[13], e[14], e[15]]), 4095);
    }

    #[test]
    fn command_upiu_is_big_endian_read() {
        let mut cdb = [0u8; 16];
        cdb[0] = 0x28;
        let u = command(7, 3, 0x1000, &cdb);
        assert_eq!(&u[0..4], &[COMMAND, FLAG_READ, 3, 7]);
        assert_eq!(&u[12..16], &[0, 0, 0x10, 0]);
        assert_eq!(u[16], 0x28);
        assert_eq!(
            command(0, 0, 0, &[0; 16])[1],
            0,
            "no data phase → no R flag"
        );
    }

    #[test]
    fn query_upiu_fields() {
        let u = query(5, QUERY_FN_READ, QOP_READ_DESC, 0x07, 2, 255);
        assert_eq!(u[0], QUERY_REQUEST);
        assert_eq!(u[3], 5);
        assert_eq!(u[5], QUERY_FN_READ);
        assert_eq!(&u[12..16], &[QOP_READ_DESC, 0x07, 2, 0]);
        assert_eq!(&u[18..20], &[0, 255]);
        assert_eq!(&u[10..12], &[0, 0], "no data-out segment");
    }

    #[test]
    fn response_header_and_sense() {
        let mut r = [0u8; 64];
        r[0] = 0x80 | RESPONSE; // HD bit set must be ignored
        r[3] = 9;
        r[6] = 0;
        r[7] = 2;
        r[10..12].copy_from_slice(&20u16.to_be_bytes());
        r[32..34].copy_from_slice(&18u16.to_be_bytes());
        r[34] = 0x70;
        let h = parse_header(&r);
        assert_eq!((h.trans, h.tag, h.status, h.data_len), (RESPONSE, 9, 2, 20));
        assert_eq!(sense(&r).len(), 18);
        r[32..34].copy_from_slice(&0xffffu16.to_be_bytes());
        assert_eq!(sense(&r).len(), 30, "length clamped to the response buffer");
    }
}
