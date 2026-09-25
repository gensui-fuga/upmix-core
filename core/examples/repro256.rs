// Reproduction of flacenc-rs issue #256: a 24-bit block that is high-entropy
// noise followed by a constant (zero) tail blows up to multi-GB output.
// Run: cargo run --release --example repro256
use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config::Encoder;
use flacenc::error::Verify;
use flacenc::source::MemSource;

const BLOCK: usize = 4096;
const BITS: usize = 24;
const FULL_SCALE: i32 = (1 << (BITS - 1)) - 1;

fn noise(n: usize) -> Vec<i32> {
    let mut state: u32 = 12345;
    (0..n)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let unit = (state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            (unit * FULL_SCALE as f32).round() as i32
        })
        .collect()
}

fn encoded_len(samples: &[i32], channels: usize) -> usize {
    let config = Encoder::default().into_verified().expect("verify");
    let source = MemSource::from_samples(samples, channels, BITS, 48_000);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .expect("encode");
    // count_bits avoids materialising a possibly multi-GB buffer
    stream.count_bits() / 8
}

fn main() {
    let pure_noise = noise(BLOCK);
    println!("pure noise block:        {} bytes", encoded_len(&pure_noise, 1));

    let mut mixed = noise(BLOCK - 1152);
    mixed.resize(BLOCK, 0);
    println!("noise + zero-tail block: {} bytes", encoded_len(&mixed, 1));

    // Also test the 6-channel interleaved version (like our upmixer output).
    let mut mixed6 = Vec::with_capacity(BLOCK * 6);
    for _ in 0..BLOCK - 1152 {
        for c in 0..6 {
            mixed6.push(noise(1)[0] + c as i32 * 1000);
        }
    }
    mixed6.resize(BLOCK * 6, 0);
    println!("6ch noise + zero tail:   {} bytes", encoded_len(&mixed6, 6));
}
