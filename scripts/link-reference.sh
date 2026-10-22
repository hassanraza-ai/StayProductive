#!/usr/bin/env bash
# Link heavy local analysis artifacts into reference/ (git-ignored).
#
#   ./scripts/link-reference.sh
#   REA_TARGETS=/path/to/analysis ./scripts/link-reference.sh
#
# Nothing is copied and nothing is committed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${REA_TARGETS:-$HOME/rea-targets}"
REF="$ROOT/reference"

mkdir -p "$REF/apk" "$REF/reports"

link() { # name target
  local dest="$REF/$1" target="$2"
  if [[ -e "$target" ]]; then
    ln -sfn "$target" "$dest"
    printf '  linked  %-18s -> %s\n' "$1" "$target"
  else
    printf '  missing %-18s    %s\n' "$1" "$target"
  fi
}

echo "reference root: $SRC"
for apk in "$SRC"/*.apk; do
  [[ -e "$apk" ]] || continue
  ln -sfn "$apk" "$REF/apk/$(basename "$apk")"
  printf '  linked  %-18s -> %s\n' "apk/$(basename "$apk")" "$apk"
done

link decompiled     "$SRC/decompiled"
link smali-project  "$SRC/build/stayfocused-smali"
link native         "$SRC/native"

for doc in REPORT.md FEATURES.md; do
  if [[ -e "$SRC/$doc" ]]; then
    ln -sfn "$SRC/$doc" "$REF/reports/$doc"
    printf '  linked  %-18s -> %s\n' "reports/$doc" "$SRC/$doc"
  fi
done

echo
echo "reference/ is git-ignored; never copy from it into tracked files."
