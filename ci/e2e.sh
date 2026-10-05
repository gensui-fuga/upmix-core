#!/usr/bin/env bash
# ci/e2e.sh —— 端到端功能测试：拿真二进制跑真音频，验输出对不对。
#
# 为什么要有它：单元测试只覆盖纯逻辑（元数据搬运、RIFF、ID3），而用户实际踩到
# 的坑全在「真二进制 + 真文件」这条路上——封面歌手歌词丢失、低音没声音、批量
# 闪退。这些只有端到端能抓到。
#
# 用法：
#   BIN=target/release/upmix-core FFMPEG=./dist-ffmpeg FFPROBE=./dist-ffprobe ci/e2e.sh
#
# 退出码：0 = 全通过；1 = 有失败；2 = 环境缺东西
#
# 刻意不用 `set -e`：一项失败不该让后面的项不跑，要一次看全所有问题。

set -uo pipefail

BIN=${BIN:-target/release/upmix-core}
FFMPEG=${FFMPEG:-dist-ffmpeg}
FFPROBE=${FFPROBE:-dist-ffprobe}

PASS=0
FAIL=0
ok()  { printf '  \033[32mPASS\033[0m %s\n' "$1"; PASS=$((PASS + 1)); }
bad() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; FAIL=$((FAIL + 1)); }
eq()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 —— 期望 [$2] 实际 [$3]"; fi; }
# gt <描述> <实际值> <下限>
gt()  { if awk -v a="$2" -v b="$3" 'BEGIN{exit !(a>b)}' 2>/dev/null; then ok "$1"; else bad "$1 —— 实测 $2，要求 > $3"; fi; }
# within <描述> <值> <下界> <上界>
within() {
  if awk -v v="$2" -v lo="$3" -v hi="$4" 'BEGIN{exit !(v>lo && v<hi)}' 2>/dev/null; then
    ok "$1（实测 $2）"
  else
    bad "$1 —— 实测 $2，要求落在 ($3, $4)"
  fi
}
section() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }

for f in "$BIN" "$FFMPEG" "$FFPROBE"; do
  if ! command -v "$f" >/dev/null 2>&1 && [ ! -x "$f" ]; then
    echo "环境不对：找不到可执行文件 $f"
    exit 2
  fi
done
BIN=$(readlink -f "$BIN")
FFMPEG=$(readlink -f "$FFMPEG")
FFPROBE=$(readlink -f "$FFPROBE")

W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
cd "$W" || exit 2

# ---------- ffprobe 小工具 ----------
chans()  { "$FFPROBE" -v error -select_streams a:0 -show_entries stream=channels    -of default=nw=1:nk=1 "$1"; }
srate()  { "$FFPROBE" -v error -select_streams a:0 -show_entries stream=sample_rate -of default=nw=1:nk=1 "$1"; }
tag()    { "$FFPROBE" -v error -show_entries format_tags="$1" -of default=nw=1:nk=1 "$2"; }
hasvid() { "$FFPROBE" -v error -select_streams v -show_entries stream=codec_type   -of default=nw=1:nk=1 "$1"; }
# 把第 N 个声道单独抽出来量平均电平（dB）。5.1 顺序 FL FR FC LFE BL BR → LFE 是 c3。
chan_db() {
  "$FFMPEG" -hide_banner -nostats -i "$1" -af "pan=mono|c0=c$2,volumedetect" -f null - 2>&1 \
    | sed -n 's/.*mean_volume: \(-\{0,1\}[0-9.]*\) dB.*/\1/p' | head -1
}

# =====================================================================
section "0. 造素材"
# =====================================================================
"$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "color=c=#3366cc:s=64x64:d=1" -frames:v 1 cover.png

# 立体声：440Hz + 60Hz，带全套标签和歌词
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:duration=4:sample_rate=44100" \
  -f lavfi -i "sine=frequency=60:duration=4:sample_rate=44100" \
  -filter_complex "[0:a][1:a]amerge=inputs=2[a]" -map "[a]" -ac 2 \
  -metadata title="测试标题" -metadata artist="测试歌手" -metadata album="测试专辑" \
  -metadata lyrics="这是测试歌词内容" stereo.flac

# 封面：把 png 当 attached_pic 塞进去
"$FFMPEG" -hide_banner -loglevel error -y -i stereo.flac -i cover.png \
  -map 0:a -map 1:v -c:a copy -c:v copy -disposition:v attached_pic \
  -metadata title="测试标题" -metadata artist="测试歌手" \
  -metadata lyrics="这是测试歌词内容" cover.flac

# MP3：ID3 + USLT 歌词
"$FFMPEG" -hide_banner -loglevel error -y -i stereo.flac \
  -c:a libmp3lame -b:a 192k -id3v2_version 3 \
  -metadata title="MP3标题" -metadata artist="MP3歌手" \
  -metadata lyrics="MP3歌词内容" song.mp3

# 24bit / 48k WAV
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:duration=3:sample_rate=48000" \
  -f lavfi -i "sine=frequency=60:duration=3:sample_rate=48000" \
  -filter_complex "[0:a][1:a]amerge=inputs=2[a]" -map "[a]" -ac 2 \
  -c:a pcm_s24le -metadata title="WAV标题" wav24.wav

# 96kHz
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:duration=2:sample_rate=96000" -ac 2 hi96.wav

# 纯低频：专门验 LFE 通道真的有能量
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=40:duration=4:sample_rate=44100" \
  -f lavfi -i "sine=frequency=45:duration=4:sample_rate=44100" \
  -filter_complex "[0:a][1:a]amerge=inputs=2[a]" -map "[a]" -ac 2 low40.flac

# 单声道 / 损坏 / 空文件
"$FFMPEG" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:duration=2:sample_rate=44100" -ac 1 mono.wav
head -c 8192 /dev/urandom > broken.flac
: > zero.wav

# .lrc 侧车歌词
printf '[00:00.00]第一行歌词\n[00:02.00]第二行歌词\n' > stereo.lrc

# --- 素材自检：素材本身不对的话，后面所有断言都是假通过 ---
eq "素材 stereo.flac 带 title"   "测试标题"         "$(tag title  stereo.flac)"
eq "素材 stereo.flac 带 artist"  "测试歌手"         "$(tag artist stereo.flac)"
eq "素材 stereo.flac 带 lyrics"  "这是测试歌词内容" "$(tag lyrics stereo.flac)"
eq "素材 cover.flac 带封面"      "video"            "$(hasvid cover.flac)"
eq "素材 song.mp3 带歌词"        "MP3歌词内容"      "$(tag lyrics song.mp3)"
eq "素材 mono.wav 是单声道"      "1"                "$(chans mono.wav)"
eq "素材 hi96.wav 是 96k"        "96000"            "$(srate hi96.wav)"

# =====================================================================
section "1. 快速模式 flac→flac：元数据必须原样搬过去"
# =====================================================================
mkdir -p o1
"$BIN" stereo.flac --mode fast --outdir o1 >/dev/null 2>&1
OUT=o1/stereo_5.1.flac
if [ -f "$OUT" ]; then ok "输出文件存在"; else bad "输出文件不存在：$OUT"; fi
eq "输出 6 声道"  "6"     "$(chans "$OUT")"
eq "采样率保持"   "44100" "$(srate "$OUT")"
eq "title 保留"   "测试标题"         "$(tag title  "$OUT")"
eq "artist 保留"  "测试歌手"         "$(tag artist "$OUT")"
eq "album 保留"   "测试专辑"         "$(tag album  "$OUT")"
eq "lyrics 保留"  "这是测试歌词内容" "$(tag lyrics "$OUT")"

# =====================================================================
section "2. 封面必须保留"
# =====================================================================
mkdir -p o2
"$BIN" cover.flac --mode fast --outdir o2 >/dev/null 2>&1
eq "输出仍有封面" "video" "$(hasvid o2/cover_5.1.flac)"

# =====================================================================
section "3. MP3 歌词（朋友报过「歌词没了」，就是这条路）"
# =====================================================================
mkdir -p o3
"$BIN" song.mp3 --mode fast --outdir o3 >/dev/null 2>&1
eq "输出歌词保留" "MP3歌词内容" "$(tag lyrics o3/song_5.1.flac)"

# =====================================================================
section "4. 24bit / 48k WAV 输入"
# =====================================================================
mkdir -p o4
"$BIN" wav24.wav --mode fast --outdir o4 >/dev/null 2>&1
eq "输出 6 声道"      "6"     "$(chans o4/wav24_5.1.flac)"
eq "采样率保持 48k"   "48000" "$(srate o4/wav24_5.1.flac)"
eq "WAV 的 title 保留" "WAV标题" "$(tag title o4/wav24_5.1.flac)"

# =====================================================================
section "5. --format wav 输出"
# =====================================================================
mkdir -p o5
"$BIN" stereo.flac --mode fast --outdir o5 --format wav >/dev/null 2>&1
W5=o5/stereo_5.1.wav
if [ -f "$W5" ]; then ok "输出 .wav 存在"; else bad "输出 .wav 不存在"; fi
eq "wav 输出 6 声道"  "6"        "$(chans "$W5")"
eq "wav title 保留"   "测试标题" "$(tag title "$W5")"

# =====================================================================
section "6. .lrc 侧车歌词（曾经被 copy(src,src) 截断成 0 字节）"
# =====================================================================
mkdir -p o6
cp stereo.flac stereo.lrc o6/
"$BIN" o6/stereo.flac --mode fast --outdir o6 >/dev/null 2>&1
LRC=o6/stereo_5.1.lrc
if [ -f "$LRC" ]; then ok "侧车歌词输出存在"; else bad "侧车歌词输出不存在：$LRC"; fi
eq "侧车歌词内容一致" "$(cat stereo.lrc)" "$(cat "$LRC" 2>/dev/null)"

# =====================================================================
section "7. 源文件绝不能被改写"
# =====================================================================
BEFORE=$(md5sum stereo.flac | cut -d' ' -f1)
mkdir -p o7
"$BIN" stereo.flac --mode fast --outdir o7 >/dev/null 2>&1
AFTER=$(md5sum stereo.flac | cut -d' ' -f1)
eq "源文件 md5 不变" "$BEFORE" "$AFTER"

# =====================================================================
section "8. 96kHz 输入保持采样率"
# =====================================================================
mkdir -p o8
"$BIN" hi96.wav --mode fast --outdir o8 >/dev/null 2>&1
eq "96k 保持" "96000" "$(srate o8/hi96_5.1.flac)"

# =====================================================================
section "9. LFE 真的有声音（朋友报过「低音没声音」）"
# =====================================================================
mkdir -p o9
"$BIN" low40.flac --mode fast --outdir o9 >/dev/null 2>&1
L=o9/low40_5.1.flac
eq "输出 6 声道" "6" "$(chans "$L")"
LFE=$(chan_db "$L" 3)
FL=$(chan_db "$L" 0)
echo "     LFE=$LFE dB   FL=$FL dB   LFE-FL=$(awk -v a="$LFE" -v b="$FL" 'BEGIN{printf "%.2f", a-b}') dB"
gt "LFE 不是静音（> -45dB）" "$LFE" "-45"
gt "主声道正常（> -20dB）"   "$FL"  "-20"
# 回归锁：默认 LFE 是 -3dB/150Hz。旧版是 -6dB/120Hz，对 40Hz 素材会低约 3dB，
# 所以这个窗口能把「改回旧参数」和「LFE 被衰减过头」都抓住。
within "LFE 相对主声道在 -3dB 附近（旧版 -6dB 会被抓）" \
  "$(awk -v a="$LFE" -v b="$FL" 'BEGIN{printf "%.3f", a-b}')" "-5" "-1"

# =====================================================================
section "10. 批量：坏文件不能拖垮整批，也不能重复处理自己的产物"
# =====================================================================
rm -rf b && mkdir -p b
cp stereo.flac low40.flac mono.wav broken.flac zero.wav b/
"$BIN" --batch b --mode fast > batch1.log 2>&1
RC=$?
eq "批量退出码 0（个别坏文件不算整批失败）" "0" "$RC"
eq "第一轮转出 2 个（mono/损坏/空 被跳过）" "2" "$(ls b/*_5.1.flac 2>/dev/null | wc -l | tr -d ' ')"
grep -q "batch done" batch1.log && ok "打印了批量汇总" || bad "没有批量汇总输出"
echo "     $(grep 'batch done' batch1.log || echo '(无)')"
"$BIN" --batch b --mode fast > batch2.log 2>&1
eq "第二轮数量不变（不把自己的 *_5.1 产物当输入）" "2" "$(ls b/*_5.1.flac 2>/dev/null | wc -l | tr -d ' ')"

# =====================================================================
section "11. 单声道必须明确报错，不能崩"
# =====================================================================
mkdir -p o11
"$BIN" mono.wav --mode fast --outdir o11 > mono.log 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then ok "非零退出码（$RC）"; else bad "单声道竟然被接受了"; fi
if grep -qi "panic" mono.log; then
  bad "出现 panic：$(grep -i panic mono.log | head -1)"
else
  ok "没有 panic"
fi
if [ -s mono.log ]; then ok "有可读的报错信息"; else bad "报错信息为空"; fi
echo "     $(head -2 mono.log | tr '\n' ' ')"

# =====================================================================
section "12. 自动分离模式（Demucs/ONNX）端到端"
# =====================================================================
mkdir -p o12
"$BIN" stereo.flac --mode auto --outdir o12 > auto.log 2>&1
RC=$?
eq "退出码 0" "0" "$RC"
eq "输出 6 声道" "6" "$(chans o12/stereo_5.1.flac)"
eq "自动模式也搬 title"  "测试标题"         "$(tag title  o12/stereo_5.1.flac)"
eq "自动模式也搬 lyrics" "这是测试歌词内容" "$(tag lyrics o12/stereo_5.1.flac)"
if [ "$RC" -ne 0 ]; then echo "     auto 日志尾部："; tail -5 auto.log | sed 's/^/       /'; fi

# =====================================================================
section "结果"
# =====================================================================
echo "  PASS=$PASS  FAIL=$FAIL"
if [ "$FAIL" -eq 0 ]; then
  printf '  \033[32m全部通过\033[0m\n'
  exit 0
fi
printf '  \033[31m有 %d 项失败\033[0m\n' "$FAIL"
exit 1
