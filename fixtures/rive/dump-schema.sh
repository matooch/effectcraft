#!/bin/sh
# Dump the Rive object model (every type's properties, keys and field types) as JSON,
# so EffectCraft's .riv reader knows how to read and skip every property.
# Run: sh fixtures/rive/dump-schema.sh
set -u
cd "$(dirname "$0")" || exit 1
mkdir -p ref/schema
rive --version > ref/schema/version.txt 2>&1
rive schema --list > ref/schema/_list.txt 2>&1
rive schema --list --json > ref/schema/_list.json 2>&1
rive schema --all --json > ref/schema/_all.json 2>&1
# One file per type too, in case --all is not what we expect.
grep -oE '^[[:space:]]*[A-Z][A-Za-z0-9]+' ref/schema/_list.txt | tr -d ' \t' | sort -u > ref/schema/_types.txt
n=0
while read -r t; do
  rive schema "$t" --json > "ref/schema/$t.json" 2>&1 && n=$((n+1))
done < ref/schema/_types.txt
echo "dumped $n types into fixtures/rive/ref/schema/ ($(wc -c < ref/schema/_all.json) bytes in _all.json)"
