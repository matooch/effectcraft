#!/bin/sh
# Build every numbered scene in this folder with the Rive CLI and capture reference
# output for EffectCraft's .riv reader tests. Run from anywhere: sh fixtures/rive/build.sh
#
# For each scene NN_name/ it writes out/NN_name/:
#   *.riv          the runtime file (unsigned, `--once`; no login needed)
#   inspect.json   the object tree as the CLI decodes it
#   summary.txt    object counts and any problems
#   rest.png       the authored pose before anything plays
#   t0000ms.png …  frames at 0.5 s steps while the default state machine plays
set -u
cd "$(dirname "$0")" || exit 1
if ! command -v rive >/dev/null 2>&1; then
  echo "rive CLI not found on PATH. Install it: https://rive.app/docs/cli/getting-started" >&2
  exit 1
fi
mkdir -p out
rive --version > out/rive-version.txt 2>&1
status=0
for dir in [0-9][0-9]_*/; do
  name=${dir%/}
  out="out/$name"
  mkdir -p "$out"
  printf '== %s\n' "$name"
  if ! rive "$name" --once --format=json > "$out/build.json" 2>&1; then
    echo "   build FAILED (see $out/build.json)"
    status=1
    continue
  fi
  find "$name" -name '*.riv' -newer "$out/build.json" -exec cp {} "$out/" \; 2>/dev/null
  find "$name" -name '*.riv' -exec cp -n {} "$out/" \; 2>/dev/null
  rive inspect "$name" --json > "$out/inspect.json" 2>&1
  rive inspect "$name" --summary > "$out/summary.txt" 2>&1
  rive "$name" --screenshot="$out/rest.png" > /dev/null 2>&1
  for ms in 17 500 1000 1500 2000; do
    rive "$name" --screenshot="$(printf '%s/t%04dms.png' "$out" "$ms")" --advance="${ms}ms" > /dev/null 2>&1 \
      || echo "   capture at ${ms}ms failed"
  done
  echo "   ok: $(ls "$out" | tr '\n' ' ')"
done
exit $status
