//! HW-11 console evidence example: render the fixed banner at 320x128,
//! print its dimensions + pinned SHA-256 digest, and write a human-viewable
//! PBM (P3) of the banner — white fg on black bg.
//!
//! Usage: cargo run --offline --example console_evidence -- [path.pbm]
//! (default output: console-banner.pbm in the current directory).

use fb_write::evidence::{frame_digest, render_banner};

const WIDTH: usize = 320;
const HEIGHT: usize = 128;

fn hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "console-banner.pbm".to_string());

    println!(
        "banner render: {}x{} (X888 LE, stride {})",
        WIDTH, HEIGHT, WIDTH
    );
    let digest = frame_digest(WIDTH, HEIGHT);
    println!("sha256 digest: {}", hex(&digest));

    // Second render must be byte-identical — determinism receipt.
    let a = render_banner(WIDTH, HEIGHT);
    let b = render_banner(WIDTH, HEIGHT);
    println!("determinism: {} (two renders identical: {})", WIDTH, a == b);

    // PBM (P3): fg white (1) where the pixel is non-black, bg black (0).
    let mut out = String::new();
    out.push_str(&format!("P3\n{} {}\n255\n", WIDTH, HEIGHT));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let px = a[y * WIDTH + x];
            let v = if px != 0 { 255 } else { 0 };
            out.push_str(&format!("{} ", v));
        }
        out.push('\n');
    }
    std::fs::write(&path, out).expect("write PBM");
    println!("wrote PBM (P3) to {}", path);
}
