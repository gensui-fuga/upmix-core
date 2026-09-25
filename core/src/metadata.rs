//! Preserve FLAC metadata (tags, cover art, lyrics) through the upmix.
//!
//! We copy every metadata block from the source except STREAMINFO (which the
//! encoder regenerates) and SEEKTABLE/CUESHEET (positions change). VORBIS_COMMENT
//! is *merged* so the original tags/lyrics survive and our channel-mask tag is
//! added on top.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const T_VORBIS_COMMENT: u8 = 4;
pub const T_PICTURE: u8 = 6;
pub const T_APPLICATION: u8 = 2;

/// A raw FLAC metadata block: (type, payload bytes).
pub type Block = (u8, Vec<u8>);

/// Read all metadata blocks from a FLAC file except STREAMINFO (type 0).
/// SEEKTABLE (3) and CUESHEET (5) are dropped because they reference offsets.
pub fn read_flac_blocks(path: &Path) -> Result<Vec<Block>> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).context("reading FLAC magic")?;
    if &magic != b"fLaC" {
        bail!("{} is not a FLAC file", path.display());
    }
    let mut blocks = Vec::new();
    loop {
        let mut head = [0u8; 4];
        f.read_exact(&mut head).context("reading metadata header")?;
        let last = head[0] & 0x80 != 0;
        let typ = head[0] & 0x7F;
        let len = ((head[1] as usize) << 16) | ((head[2] as usize) << 8) | head[3] as usize;
        let mut data = vec![0u8; len];
        f.read_exact(&mut data).context("reading metadata body")?;
        match typ {
            0 | 3 | 5 => {} // STREAMINFO / SEEKTABLE / CUESHEET: skip
            _ => blocks.push((typ, data)),
        }
        if last {
            break;
        }
    }
    Ok(blocks)
}

/// Parse a Vorbis Comment block payload into its `KEY=value` comment strings.
fn parse_vorbis(payload: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let rd_u32 = |p: &mut usize| -> Option<u32> {
        if *p + 4 > payload.len() {
            return None;
        }
        let v = u32::from_le_bytes([payload[*p], payload[*p + 1], payload[*p + 2], payload[*p + 3]]);
        *p += 4;
        Some(v)
    };
    // vendor
    let Some(vlen) = rd_u32(&mut p) else { return out };
    p = (p + vlen as usize).min(payload.len());
    let Some(count) = rd_u32(&mut p) else { return out };
    for _ in 0..count {
        let Some(clen) = rd_u32(&mut p) else { break };
        let end = (p + clen as usize).min(payload.len());
        if let Ok(s) = std::str::from_utf8(&payload[p..end]) {
            out.push(s.to_string());
        }
        p = end;
    }
    out
}

/// Serialize a Vorbis Comment block payload from comment strings.
fn build_vorbis(vendor: &str, comments: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor.as_bytes());
    out.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(c.as_bytes());
    }
    out
}

/// Rebuild the metadata block list for an output file.
///
/// * Keeps every PICTURE / APPLICATION block from `source`.
/// * Merges the source VORBIS_COMMENT comments with `extra_tags` (e.g. the
///   channel mask), so tags and lyrics are preserved.
pub fn build_output_blocks(source: &[Block], extra_tags: &[String]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut comments: Vec<String> = Vec::new();

    for (typ, data) in source {
        match *typ {
            T_VORBIS_COMMENT => comments.extend(parse_vorbis(data)),
            T_PICTURE | T_APPLICATION => out.push((*typ, data.clone())),
            _ => {}
        }
    }
    for t in extra_tags {
        if !comments.iter().any(|c| c.eq_ignore_ascii_case(t)) {
            comments.push(t.clone());
        }
    }
    out.insert(0, (T_VORBIS_COMMENT, build_vorbis("upmix-core", &comments)));
    out
}

/// Rewrite the metadata section of an already-encoded FLAC file: keep its
/// STREAMINFO and audio frames, but replace every other metadata block with the
/// merged set from `source` plus `extra_tags`. This is how tags, cover art and
/// lyrics survive into the upmixed file, regardless of which encoder ran.
pub fn inject_into_flac(path: &Path, source: Option<&Path>, extra_tags: &[String]) -> Result<()> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if data.len() < 4 || &data[0..4] != b"fLaC" {
        bail!("{} is not a FLAC file", path.display());
    }

    let mut pos = 4usize;
    let mut streaminfo: Option<Vec<u8>> = None;
    loop {
        if pos + 4 > data.len() {
            bail!("truncated FLAC metadata");
        }
        let head = &data[pos..pos + 4];
        let last = head[0] & 0x80 != 0;
        let typ = head[0] & 0x7F;
        let len = ((head[1] as usize) << 16) | ((head[2] as usize) << 8) | head[3] as usize;
        let body_start = pos + 4;
        let body_end = body_start + len;
        if body_end > data.len() {
            bail!("truncated FLAC metadata body");
        }
        if typ == 0 {
            streaminfo = Some(data[body_start..body_end].to_vec());
        }
        pos = body_end;
        if last {
            break;
        }
    }
    let audio = &data[pos..];
    let si = streaminfo.ok_or_else(|| anyhow::anyhow!("FLAC has no STREAMINFO"))?;

    let src_blocks = source
        .and_then(|p| read_flac_blocks(p).ok())
        .unwrap_or_default();
    let blocks = build_output_blocks(&src_blocks, extra_tags);

    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(b"fLaC");
    // STREAMINFO
    let si_last = blocks.is_empty();
    out.push(if si_last { 0x80 } else { 0x00 });
    out.extend_from_slice(&[(si.len() >> 16) as u8, (si.len() >> 8) as u8, si.len() as u8]);
    out.extend_from_slice(&si);
    // our blocks
    for (i, (typ, payload)) in blocks.iter().enumerate() {
        let last = i == blocks.len() - 1;
        out.push(if last { 0x80 } else { 0x00 } | (typ & 0x7F));
        out.extend_from_slice(&[(payload.len() >> 16) as u8, (payload.len() >> 8) as u8, payload.len() as u8]);
        out.extend_from_slice(payload);
    }
    out.extend_from_slice(audio);
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorbis_roundtrip() {
        let comments = vec!["TITLE=Test".to_string(), "ARTIST=Ye".to_string()];
        let payload = build_vorbis("upmix-core", &comments);
        assert_eq!(parse_vorbis(&payload), comments);
    }

    #[test]
    fn merge_keeps_source_tags_and_adds_mask() {
        let src_vc = build_vorbis("ref", &["TITLE=Song".to_string(), "LYRICS=la la".to_string()]);
        let source: Vec<Block> = vec![(T_VORBIS_COMMENT, src_vc), (T_PICTURE, vec![1, 2, 3])];
        let out = build_output_blocks(&source, &["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x003F".to_string()]);
        // picture preserved
        assert!(out.iter().any(|(t, _)| *t == T_PICTURE));
        // vorbis merged
        let vc = out.iter().find(|(t, _)| *t == T_VORBIS_COMMENT).unwrap();
        let tags = parse_vorbis(&vc.1);
        assert!(tags.iter().any(|c| c == "TITLE=Song"));
        assert!(tags.iter().any(|c| c == "LYRICS=la la"));
        assert!(tags.iter().any(|c| c.contains("CHANNEL_MASK")));
    }
}
