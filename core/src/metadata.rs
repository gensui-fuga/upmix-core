//! 元数据保留：标签、封面、歌词，**所有输入格式**通吃。
//!
//! 以前这里只认 FLAC：源不是 FLAC 就 `bail!`，调用方还把这个错误 `.ok()`
//! 吞掉，于是 mp3/wav/m4a/ogg/opus 输入的输出只剩一个声道掩码标签。
//! 现在改成两段式：
//!
//! 1. **抽取**：用自带的 ffmpeg 把任意源文件抽成一个几百毫秒的“元数据捐赠
//!    者”FLAC，标签和封面都在里面。为什么用 FLAC 当载体：本文件已经有一套
//!    完整、被测过的 FLAC 元数据搬运代码，复用它就不用再给 mp3/m4a/ogg 各写
//!    一套标签解析器，也不用引入新依赖。
//! 2. **注入**：把捐赠者的块注入到输出。
//!    * 输出 FLAC → 替换元数据块（原有逻辑）。
//!    * 输出 WAV → 追加 RIFF `LIST/INFO` 块，另外把 FLAC 专属的键（歌词等）
//!      塞进 ID3v2 风格的 `id3 ` 块，否则 WAV 会丢歌词。
//!
//! 关于标签在哪一层（实测，别再踩）：
//!   * flac / wav / mp3 / m4a → 标签在 **format(global)** 层，`-map_metadata 0`
//!   * ogg / opus            → 标签在 **stream** 层，必须 `-map_metadata:g 0:s:a:0`
//!   两个都写上才能全覆盖。

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

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
pub fn parse_vorbis(payload: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let rd_u32 = |p: &mut usize| -> Option<u32> {
        if *p + 4 > payload.len() {
            return None;
        }
        let v = u32::from_le_bytes([
            payload[*p],
            payload[*p + 1],
            payload[*p + 2],
            payload[*p + 3],
        ]);
        *p += 4;
        Some(v)
    };
    // vendor
    let Some(vlen) = rd_u32(&mut p) else {
        return out;
    };
    p = (p + vlen as usize).min(payload.len());
    let Some(count) = rd_u32(&mut p) else {
        return out;
    };
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

/// 容器自己的技术键，不是用户标签。m4a 尤其爱往外吐这些：抽捐赠者时
/// `-map_metadata` 会把它们一起搬过来，不滤掉用户输出里就会多出
/// `handler_name=SoundHandler` / `language=und` 这种垃圾。
const CONTAINER_NOISE: &[&str] = &[
    "major_brand",
    "minor_version",
    "compatible_brands",
    "handler_name",
    "vendor_id",
    "language",
    "creation_time",
];

fn is_container_noise(comment: &str) -> bool {
    match comment.split_once('=') {
        Some((k, _)) => CONTAINER_NOISE.contains(&k.trim().to_ascii_lowercase().as_str()),
        None => true, // 没有 '=' 的畸形条目直接丢
    }
}

/// 从捐赠者的元数据块里摘出 `KEY=value` 标签（不含封面，也不含容器噪声）。
pub fn tags_from_blocks(blocks: &[Block]) -> Vec<String> {
    let mut comments = Vec::new();
    for (typ, data) in blocks {
        if *typ == T_VORBIS_COMMENT {
            comments.extend(parse_vorbis(data));
        }
    }
    comments.retain(|c| !is_container_noise(c));
    comments
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
            T_VORBIS_COMMENT => {
                // 同样要滤容器噪声，否则 m4a 会把 handler_name 带进 FLAC。
                let mut c = parse_vorbis(data);
                c.retain(|x| !is_container_noise(x));
                comments.extend(c);
            }
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

    // 源元数据读不到时不再静默降级成“空元数据”——那正是标签全丢的成因。
    // 但我们也不能因此让整次转换失败：音频已经编码好了，丢掉它更糟。所以
    // 退化成“只写自己的标签 + 打一条警告”。
    let src_blocks = match source {
        Some(p) => match load_source_blocks(p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "warning: 读不到 {} 的元数据（{e}），输出将不含原标签",
                    p.display()
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let blocks = build_output_blocks(&src_blocks, extra_tags);

    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(b"fLaC");
    // STREAMINFO
    let si_last = blocks.is_empty();
    out.push(if si_last { 0x80 } else { 0x00 });
    out.extend_from_slice(&[
        (si.len() >> 16) as u8,
        (si.len() >> 8) as u8,
        si.len() as u8,
    ]);
    out.extend_from_slice(&si);
    // our blocks
    for (i, (typ, payload)) in blocks.iter().enumerate() {
        let last = i == blocks.len() - 1;
        out.push(if last { 0x80 } else { 0x00 } | (typ & 0x7F));
        out.extend_from_slice(&[
            (payload.len() >> 16) as u8,
            (payload.len() >> 8) as u8,
            payload.len() as u8,
        ]);
        out.extend_from_slice(payload);
    }
    out.extend_from_slice(audio);
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// =====================================================================
// 任意格式的元数据抽取（“捐赠者”机制）
// =====================================================================

/// 找一个 ffmpeg：优先程序自己旁边的（发行包随机附带），再 PATH。
///
/// 这里曾经是 `fileio::ffmpeg_path()` 的一份复制粘贴。同一件事有两份实现，
/// 迟早会分叉（这次修 bug 的教训）——所以直接复用。
pub fn ffmpeg_bin() -> PathBuf {
    crate::fileio::ffmpeg_path()
}

fn unique_temp(tag: &str, ext: &str) -> PathBuf {
    // PID + 纳秒时间戳：同一个进程里连着转好几个文件也不会互相踩。
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("upmix-{tag}-{}-{nanos}.{ext}", std::process::id()))
}

/// 把任意格式的源抽成一个极小的“元数据捐赠者”FLAC（0.1 秒音频 + 全部标签 +
/// 内嵌封面）。实测 0.11 秒、10~21 KB。
///
/// 两个 `-map_metadata` 都必要：一个搬 global 层（flac/wav/mp3/m4a），一个把
/// 源 stream 层提到输出 global 层（ogg/opus，它们的标签只在流上）。
/// `-map 0:v:0?` 末尾的 `?` 表示“没有视频流也别报错”。
pub fn make_metadata_donor(src: &Path) -> Result<PathBuf> {
    let out = unique_temp("meta", "flac");
    let ff = ffmpeg_bin();
    let res = std::process::Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(src)
        .args([
            "-map",
            "0:a",
            "-map_metadata",
            "0",
            "-map_metadata:g",
            "0:s:a:0",
            "-map",
            "0:v:0?",
            "-c:a",
            "flac",
            "-c:v",
            "copy",
            "-disposition:v",
            "attached_pic",
            "-t",
            "0.1",
        ])
        .arg(&out)
        .output();
    match res {
        Ok(o) if o.status.success() && out.exists() => Ok(out),
        Ok(o) => {
            let _ = std::fs::remove_file(&out);
            bail!(
                "ffmpeg 抽不出 {} 的元数据：{}",
                src.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            )
        }
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            bail!("调用 ffmpeg 失败（{e}）")
        }
    }
}

/// 读任意格式源的元数据块：FLAC 直接读，其他格式先抽捐赠者再读。
pub fn load_source_blocks(src: &Path) -> Result<Vec<Block>> {
    let is_flac = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("flac"))
        .unwrap_or(false);
    if is_flac {
        // FLAC 源不需要绕 ffmpeg，既快又不依赖外部程序。
        return read_flac_blocks(src);
    }
    let donor = make_metadata_donor(src)?;
    let blocks = read_flac_blocks(&donor);
    let _ = std::fs::remove_file(&donor);
    blocks
}

// =====================================================================
// WAV：RIFF LIST/INFO（+ 歌词走 id3 块）
// =====================================================================

/// 标签键 → RIFF INFO 四字键（Info 规范）。
fn riff_info_key(tag: &str) -> Option<[u8; 4]> {
    let k = tag.split('=').next()?.trim().to_ascii_uppercase();
    let v = match k.as_str() {
        "TITLE" => b"INAM",
        "ARTIST" | "ALBUMARTIST" | "ALBUM_ARTIST" => b"IART",
        "ALBUM" => b"IPRD",
        "GENRE" => b"IGNR",
        "DATE" | "YEAR" => b"ICRD",
        "TRACKNUMBER" | "TRACK" => b"ITRK",
        "COMMENT" | "DESCRIPTION" => b"ICMT",
        "COPYRIGHT" => b"ICOP",
        "ENCODER" | "ENCODED-BY" | "ENCODED_BY" => b"ISFT",
        // WAV 没有标准的“歌词”键，下面单独走 id3 块。
        _ => return None,
    };
    Some(*v)
}

/// 需要塞进 WAV 的 `id3 ` 块的标签（RIFF INFO 表达不了的）。
fn id3_only_tags(comments: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in comments {
        let Some((k, v)) = c.split_once('=') else {
            continue;
        };
        let ku = k.trim().to_ascii_uppercase();
        // 歌词/作词作曲这类 RIFF INFO 没有对应键的，全过继给 ID3。
        let keep = matches!(
            ku.as_str(),
            "LYRICS"
                | "UNSYNCEDLYRICS"
                | "UNSYNCED_LYRICS"
                | "SYNCEDLYRICS"
                | "COMPOSER"
                | "LYRICIST"
        );
        if keep {
            out.push((ku, v.to_string()));
        }
    }
    out
}

fn id3_text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut body = vec![0x03u8]; // UTF-8
    body.extend_from_slice(text.as_bytes());
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0, 0]); // flags
    out.extend_from_slice(&body);
    out
}

/// 组一个 ID3v2.3 tag（作为 WAV 的 `id3 ` 块）。只装歌词类帧。
fn build_id3v23(pairs: &[(String, String)]) -> Vec<u8> {
    let mut frames = Vec::new();
    for (k, v) in pairs {
        let id: Option<[u8; 4]> = match k.as_str() {
            "LYRICS" | "UNSYNCEDLYRICS" | "UNSYNCED_LYRICS" => Some(*b"USLT"),
            "SYNCEDLYRICS" => Some(*b"SYLT"),
            "COMPOSER" | "LYRICIST" => Some(*b"TCOM"),
            _ => None,
        };
        if let Some(id) = id {
            if id == *b"USLT" {
                // USLT: encoding(1) + lang(3) + descriptor(NUL) + text
                let mut body = vec![0x03u8];
                body.extend_from_slice(b"eng");
                body.push(0);
                body.extend_from_slice(v.as_bytes());
                frames.extend_from_slice(&id);
                frames.extend_from_slice(&(body.len() as u32).to_be_bytes());
                frames.extend_from_slice(&[0, 0]);
                frames.extend_from_slice(&body);
            } else {
                frames.extend_from_slice(&id3_text_frame(&id, v));
            }
        }
    }
    if frames.is_empty() {
        return Vec::new();
    }
    // ID3v2.3 header: "ID3" + ver(2) + flags(1) + 同步安全长度(4)
    let size = frames.len() as u32;
    let ss = [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ];
    let mut out = Vec::new();
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[0x03, 0x00, 0x00]);
    out.extend_from_slice(&ss);
    out.extend_from_slice(&frames);
    out
}

fn riff_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0); // RIFF 要求偶数长度，补位字节不计入 size
    }
    out
}

/// 往已经写好的 WAV 文件里追加标签。
///
/// hound 只写 `fmt ` + `data`，标签得自己加。做法是**追加**而不是重写：在
/// 文件末尾补一个 `LIST/INFO`（还有需要的话一个 `id3 `），读 WAV 的人会一路
/// 扫到 EOF，实测 ffmpeg / ffprobe 都能正常读出来。
///
/// 封面不通往 WAV：wav muxer 完全不支持视频流，硬塞会让 ffmpeg 写出 0 字节
/// 文件。所以这里只搬标签。
pub fn inject_into_wav(path: &Path, source: Option<&Path>) -> Result<()> {
    let Some(src) = source else { return Ok(()) };
    let blocks = match load_source_blocks(src) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "warning: 读不到 {} 的元数据（{e}），输出将不含原标签",
                src.display()
            );
            return Ok(());
        }
    };
    let comments = tags_from_blocks(&blocks);
    if comments.is_empty() {
        return Ok(());
    }

    // LIST/INFO 主体
    let mut info = Vec::new();
    info.extend_from_slice(b"INFO");
    for c in &comments {
        let Some((k, v)) = c.split_once('=') else {
            continue;
        };
        if let Some(key) = riff_info_key(k) {
            if v.is_empty() {
                continue;
            }
            let mut val = v.as_bytes().to_vec();
            val.push(0); // INFO 值是 NUL 结尾字符串
            info.extend_from_slice(&riff_chunk(&key, &val));
        }
    }

    let mut extra = Vec::new();
    if info.len() > 4 {
        extra.extend_from_slice(&riff_chunk(b"LIST", &info));
    }

    // 歌词类 → id3 块（WAV 的 ID3 约定就是四字键 "id3 "）
    let pairs = id3_only_tags(&comments);
    if !pairs.is_empty() {
        let id3 = build_id3v23(&pairs);
        if !id3.is_empty() {
            extra.extend_from_slice(&riff_chunk(b"id3 ", &id3));
        }
    }

    if extra.is_empty() {
        return Ok(());
    }

    // data chunk 若为奇数长度，RIFF 规定要补一个字节；hound 不补，我们补上，
    // 否则后面的块会错位。
    let mut need_pad = false;
    {
        let d = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if d.len() >= 12 {
            let mut p = 12usize;
            while p + 8 <= d.len() {
                let id = &d[p..p + 4];
                let sz = u32::from_le_bytes([d[p + 4], d[p + 5], d[p + 6], d[p + 7]]) as usize;
                if id == b"data" {
                    need_pad = sz % 2 == 1;
                    break;
                }
                p += 8 + sz + (sz & 1);
            }
        }
    }

    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("opening {} for append", path.display()))?;
    use std::io::Write;
    if need_pad {
        f.write_all(&[0]).context("padding WAV data chunk")?;
    }
    f.write_all(&extra).context("appending WAV tags")?;
    f.flush().ok();
    Ok(())
}

// =====================================================================
// 侧车歌词（.lrc）
// =====================================================================

/// 输入旁边如果有同名 `.lrc`，把它拷到输出旁边（改成输出的名字）。
///
/// 播放器是按“音频文件名”去找歌词的：`song_5.1.flac` 要配 `song_5.1.lrc`。
/// 不做这一步，用户放在旁边的歌词就成了孤儿。
pub fn copy_sidecar_lyrics(output: &Path, source: &Path) {
    let Some(src_dir) = source.parent() else {
        return;
    };
    let Some(src_stem) = source.file_stem().and_then(|s| s.to_str()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(if src_dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        src_dir
    }) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let is_lrc = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("lrc"))
            .unwrap_or(false);
        if !is_lrc {
            continue;
        }
        // 同名（含大小写宽松比较）才算这个音频的歌词。
        let matched = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(src_stem))
            .unwrap_or(false);
        if !matched {
            continue;
        }
        let Some(out_stem) = output.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let dest = match output.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.join(format!("{out_stem}.lrc")),
            _ => PathBuf::from(format!("{out_stem}.lrc")),
        };
        if dest == p {
            continue; // 同名同地（比如原地覆盖），不用动
        }
        if let Err(err) = std::fs::copy(&p, &dest) {
            eprintln!("warning: 复制歌词 {} 失败（{err}）", p.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorbis_roundtrip() {
        let comments = vec!["TITLE=Song".to_string(), "LYRICS=la la".to_string()];
        let payload = build_vorbis("upmix-core", &comments);
        assert_eq!(parse_vorbis(&payload), comments);
    }

    #[test]
    fn merge_keeps_source_tags_and_adds_mask() {
        let src_vc = build_vorbis(
            "ref",
            &["TITLE=Song".to_string(), "LYRICS=la la".to_string()],
        );
        let source: Vec<Block> = vec![(T_VORBIS_COMMENT, src_vc), (T_PICTURE, vec![1, 2, 3])];
        let out = build_output_blocks(
            &source,
            &["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x003F".to_string()],
        );
        // picture preserved
        assert!(out.iter().any(|(t, _)| *t == T_PICTURE));
        // vorbis merged
        let vc = out.iter().find(|(t, _)| *t == T_VORBIS_COMMENT).unwrap();
        let tags = parse_vorbis(&vc.1);
        assert!(tags.iter().any(|c| c == "TITLE=Song"));
        assert!(tags.iter().any(|c| c == "LYRICS=la la"));
        assert!(tags.iter().any(|c| c.contains("CHANNEL_MASK")));
    }

    #[test]
    fn riff_keys_map() {
        assert_eq!(riff_info_key("TITLE"), Some(*b"INAM"));
        assert_eq!(riff_info_key("artist"), Some(*b"IART"));
        assert_eq!(riff_info_key("ALBUM"), Some(*b"IPRD"));
        assert_eq!(riff_info_key("LYRICS"), None); // 歌词走 id3
    }

    #[test]
    fn id3_block_carries_lyrics() {
        let blocks: Vec<Block> = vec![(
            T_VORBIS_COMMENT,
            build_vorbis("t", &["TITLE=S".to_string(), "LYRICS=hello".to_string()]),
        )];
        let pairs = id3_only_tags(&tags_from_blocks(&blocks));
        assert_eq!(pairs.len(), 1);
        let id3 = build_id3v23(&pairs);
        assert!(id3.starts_with(b"ID3"));
        assert!(id3.windows(4).any(|w| w == b"USLT"));
        assert!(id3.windows(5).any(|w| w == b"hello"));
    }

    #[test]
    fn riff_chunk_pads_odd_body() {
        let c = riff_chunk(b"INAM", b"abc");
        assert_eq!(c.len(), 4 + 4 + 4); // id + size + 3 字节 + 1 补位
        assert_eq!(u32::from_le_bytes([c[4], c[5], c[6], c[7]]), 3);
    }

    #[test]
    fn container_noise_is_filtered() {
        assert!(is_container_noise("handler_name=SoundHandler"));
        assert!(is_container_noise("major_brand=M4A "));
        assert!(is_container_noise("language=und"));
        assert!(!is_container_noise("TITLE=Song"));
        assert!(!is_container_noise("artist=Some Artist"));
        // 过滤后 m4a 的垃圾不会进输出
        let vc = build_vorbis(
            "t",
            &[
                "handler_name=S".to_string(),
                "title=T".to_string(),
                "language=und".to_string(),
            ],
        );
        let tags = tags_from_blocks(&[(T_VORBIS_COMMENT, vc)]);
        assert_eq!(tags, vec!["title=T".to_string()]);
    }
}
