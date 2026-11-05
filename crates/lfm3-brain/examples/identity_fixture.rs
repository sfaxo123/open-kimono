//! Reproduce the frozen synthetic identity fixture. No hashing or model weights.
//! Compare its output using the retained host SHA256 comparison oracle.
fn payload(length: usize) -> Vec<u8> {
    let mut state = 0x9E3779B97F4A7C15_u64;
    (0..length)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("expected output file path")?;
    let header = br#"{"weight":{"dtype":"F32","shape":[3,2],"data_offsets":[0,24]},"bias":{"dtype":"F32","shape":[2],"data_offsets":[24,32]}}"#;
    let mut file = (header.len() as u64).to_le_bytes().to_vec();
    file.extend_from_slice(header);
    file.extend(payload(24));
    file.extend(payload(8));
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&file)?;
    Ok(())
}
