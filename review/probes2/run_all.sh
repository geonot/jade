#!/usr/bin/env bash
# v2 probe runner — also strips inline // (Jinn only supports #)
set -u
cd "$(dirname "$0")"
JINNC=${JINNC:-../../target/release/jinnc}
RESULTS=${RESULTS:-/tmp/jinn-probes2.log}
: > "$RESULTS"
shopt -s nullglob
for src in *.jn; do
    base="${src%.jn}"
    bin="/tmp/jinn-p2-$base"
    rm -f "$bin"
    echo "===== $src =====" >> "$RESULTS"
    cstart=$(date +%s%N)
    cout=$("$JINNC" "$src" -o "$bin" 2>&1)
    crc=$?
    cend=$(date +%s%N)
    cms=$(( (cend - cstart) / 1000000 ))
    echo "COMPILE rc=$crc  ${cms}ms" >> "$RESULTS"
    if [ -n "$cout" ]; then
        printf -- "--- compile output ---\n%s\n" "$cout" >> "$RESULTS"
    fi
    if [ $crc -eq 0 ]; then
        rstart=$(date +%s%N)
        rout=$(timeout 10 "$bin" 2>&1)
        rrc=$?
        rend=$(date +%s%N)
        rms=$(( (rend - rstart) / 1000000 ))
        echo "RUN     rc=$rrc  ${rms}ms" >> "$RESULTS"
        printf -- "--- run output ---\n%s\n" "$rout" >> "$RESULTS"
    fi
    echo "" >> "$RESULTS"
done
echo "Wrote $RESULTS"
