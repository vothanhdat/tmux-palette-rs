#!/usr/bin/env bash
# Build the vertical short (YouTube Shorts / TikTok / Reels): 1080x1920, 30 fps,
# H.264 + a silent AAC track, written to docs/media/short.mp4.
#
#   demo/short/make.sh             # record the scene clips, then compose
#   demo/short/make.sh --compose   # re-compose from the clips already recorded
#
# Each scene is a real recording (demo/short/<n>-*.tape, via VHS and the same
# throwaway workspace as the other demos, see demo/README.md), placed under a
# caption on a backdrop. The captions, backdrop and closing card are
# demo/short/cards.html, rendered to PNG with headless Chromium; ffmpeg
# composes the rest. Needs everything demo/record.sh needs, plus Chromium or
# Chrome (set CHROME=/path/to/chrome if it isn't found).
set -euo pipefail

cd "$(dirname "$0")/../.."

OUT=docs/media/short.mp4
WORK=target/short
SCENES=(1-hook 2-jump 3-find 4-command 5-themes 6-custom)
SLOT_X=24 SLOT_Y=500 # the terminal's place in cards.html (--slot-x/--slot-y)
FPS=30
CAPTION_IN=0.35 # caption fade/slide-in, seconds
OUTRO=4.5       # closing card, seconds
XFADE=0.5       # crossfade into the closing card

find_chrome() {
  for c in "${CHROME:-}" chromium chromium-browser google-chrome google-chrome-stable \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"; do
    [ -n "$c" ] && command -v "$c" >/dev/null && { echo "$c"; return; }
  done
  echo "short: Chromium/Chrome not found; set CHROME=/path/to/chrome" >&2
  exit 1
}
CHROME_BIN="$(find_chrome)"
for tool in ffmpeg ffprobe; do
  command -v "$tool" >/dev/null || { echo "short: '$tool' not found" >&2; exit 1; }
done

if [ "${1:-}" != "--compose" ]; then
  for tool in vhs tmux git; do
    command -v "$tool" >/dev/null || { echo "short: '$tool' not found" >&2; exit 1; }
  done
  cargo build --release --quiet
  mkdir -p "$WORK"
  for s in "${SCENES[@]}"; do
    echo "==> recording $s"
    vhs "demo/short/$s.tape" >/dev/null
  done
  demo/env/setup.sh --kill
fi

# --- stills ------------------------------------------------------------------

mkdir -p "$WORK/cards"
render() { # render <hash> <out.png> [WxH]
  local sandbox=()
  [ "$(id -u)" = 0 ] && sandbox=(--no-sandbox) # Chromium refuses root otherwise
  "$CHROME_BIN" --headless=new "${sandbox[@]}" --disable-gpu --hide-scrollbars \
    --force-device-scale-factor=1 --default-background-color=00000000 \
    --window-size="${3:-1080,1920}" --virtual-time-budget=2000 \
    --screenshot="$2" "file://$PWD/demo/short/cards.html#$1" >/dev/null 2>&1
  [ -s "$2" ] || { echo "short: failed to render #$1" >&2; exit 1; }
}
echo "==> rendering cards"
render bg "$WORK/cards/bg.png"
render outro "$WORK/cards/outro.png"
render mask "$WORK/cards/mask.png" 1032,1120
for i in "${!SCENES[@]}"; do
  render "cap=$((i + 1))" "$WORK/cards/cap-$((i + 1)).png"
done

# --- segments ----------------------------------------------------------------

duration() { ffprobe -v error -show_entries format=duration -of csv=p=0 "$1"; }
calc() { awk "BEGIN { print $* }"; }
encode=(-c:v libx264 -preset veryfast -crf 12 -pix_fmt yuv420p -r "$FPS" -an)

echo "==> composing"
segments=()
total=0
for i in "${!SCENES[@]}"; do
  n=$((i + 1))
  clip="$WORK/${SCENES[$i]}.mp4"
  [ -f "$clip" ] || { echo "short: $clip missing; run without --compose" >&2; exit 1; }
  d="$(duration "$clip")"
  # The first caption is there from frame one: it is the thumbnail.
  if [ "$n" = 1 ]; then
    cap="[3]format=rgba[c];[a][c]overlay=0:0"
  else
    cap="[3]format=rgba,fade=t=in:st=0:d=$CAPTION_IN:alpha=1[c];"
    cap+="[a][c]overlay=x=0:y='26*pow(1-min(t/$CAPTION_IN,1),2)':eval=frame"
  fi
  ffmpeg -y -loglevel error \
    -loop 1 -framerate "$FPS" -t "$d" -i "$WORK/cards/bg.png" \
    -i "$clip" \
    -loop 1 -framerate "$FPS" -t "$d" -i "$WORK/cards/mask.png" \
    -loop 1 -framerate "$FPS" -t "$d" -i "$WORK/cards/cap-$n.png" \
    -filter_complex "[1]fps=$FPS,format=rgba[v];[2]format=gray[m];[v][m]alphamerge[t];
      [0][t]overlay=$SLOT_X:$SLOT_Y:shortest=1[a];$cap,format=yuv420p" \
    -t "$d" "${encode[@]}" "$WORK/seg-$n.mp4"
  segments+=("$WORK/seg-$n.mp4")
  total="$(calc "$total + $(duration "$WORK/seg-$n.mp4")")"
done

ffmpeg -y -loglevel error -loop 1 -framerate "$FPS" -t "$OUTRO" -i "$WORK/cards/outro.png" \
  -vf format=yuv420p "${encode[@]}" "$WORK/seg-outro.mp4"

# --- final -------------------------------------------------------------------

inputs=()
chain=""
for i in "${!segments[@]}"; do
  inputs+=(-i "${segments[$i]}")
  chain+="[$i:v]"
done
k=${#segments[@]}
offset="$(calc "$total - $XFADE")"
length="$(calc "$total + $OUTRO - $XFADE")"

mkdir -p "$(dirname "$OUT")"
ffmpeg -y -loglevel error "${inputs[@]}" -i "$WORK/seg-outro.mp4" \
  -f lavfi -t "$length" -i "anullsrc=channel_layout=stereo:sample_rate=48000" \
  -filter_complex "${chain}concat=n=$k:v=1:a=0,settb=AVTB,fps=$FPS[main];
    [$k:v]settb=AVTB,fps=$FPS[end];
    [main][end]xfade=transition=fade:duration=$XFADE:offset=$offset,format=yuv420p[v]" \
  -map "[v]" -map "$((k + 1)):a" \
  -c:v libx264 -preset slow -crf 20 -profile:v high -level 4.2 -pix_fmt yuv420p -r "$FPS" \
  -c:a aac -b:a 128k -shortest -movflags +faststart "$OUT"

printf '%s: %ss, %s\n' "$OUT" "$(printf '%.1f' "$(duration "$OUT")")" "$(du -h "$OUT" | cut -f1)"
