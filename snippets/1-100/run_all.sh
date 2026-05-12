#!/usr/bin/env bash
# Run every s*.jn snippet and report exit/output briefly.
set -u
JINN=/home/rome/Glitch/software/jade/target/release/jinnc
OUT_DIR=/tmp/jinn_snippets/out
mkdir -p "$OUT_DIR"
total=0
ok=0
compile_err=0
runtime_err=0
results=/tmp/jinn_snippets/results.txt
: > "$results"
for f in /tmp/jinn_snippets/s*.jn; do
    total=$((total+1))
    name=$(basename "$f" .jn)
    bin="$OUT_DIR/$name"
    rm -f "$bin"
    cerr=$("$JINN" "$f" -o "$bin" 2>&1)
    ccode=$?
    if [ $ccode -ne 0 ] || [ ! -x "$bin" ]; then
        compile_err=$((compile_err+1))
        echo "===[CE]=== $name" >> "$results"
        echo "$cerr" | head -8 >> "$results"
        echo "---" >> "$results"
        continue
    fi
    rout=$("$bin" 2>&1)
    rcode=$?
    if [ $rcode -ne 0 ]; then
        runtime_err=$((runtime_err+1))
        echo "===[RE:$rcode]=== $name" >> "$results"
        echo "$rout" | head -8 >> "$results"
        echo "---" >> "$results"
    else
        ok=$((ok+1))
        echo "===[OK]=== $name" >> "$results"
        echo "$rout" | head -4 >> "$results"
        echo "---" >> "$results"
    fi
done
echo "Total=$total OK=$ok CompileErr=$compile_err RuntimeErr=$runtime_err"
