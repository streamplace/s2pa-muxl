#!/usr/bin/env bash
set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <input.mp4>"
    exit 1
fi

INPUT="$(realpath "$1")"
OUTDIR="$(dirname "$INPUT")/output"
mkdir -p "$OUTDIR"

echo "=== Remuxing $INPUT ==="
echo

echo "--- ffmpeg ---"
ffmpeg -y -i "$INPUT" -c copy "$OUTDIR/ffmpeg.mp4" 2>&1 | tail -1
echo

echo "--- ffmpeg (faststart) ---"
ffmpeg -y -i "$INPUT" -c copy -movflags +faststart "$OUTDIR/ffmpeg-faststart.mp4" 2>&1 | tail -1
echo

echo "--- gstreamer ---"
gst-launch-1.0 -e \
    filesrc location="$INPUT" ! qtdemux name=demux \
    mp4mux name=mux ! filesink location="$OUTDIR/gstreamer.mp4" \
    demux.video_0 ! queue ! mux.video_0 \
    demux.audio_0 ! queue ! mux.audio_0
echo

echo "--- MP4Box ---"
MP4Box -add "$INPUT" -new "$OUTDIR/mp4box.mp4"
echo

echo "=== Results ==="
echo
echo "File sizes:"
ls -l "$OUTDIR"/*.mp4 | awk '{print $5, $NF}'
echo
echo "MD5 sums:"
md5sum "$OUTDIR"/*.mp4
