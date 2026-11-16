#!/bin/bash
# Uploads, downloads and reads back a cookbook through the signed API. Every
# step asserts; the script only exits 0 when the whole round trip worked.
set -euo pipefail

KNIFE=(knife supermarket)
CONFIG=(-c /work/knife.rb)
VERSION=0.1.0
ARCHIVE=/tmp/demo-$VERSION.tar.gz

step() { printf '\n==> %s\n' "$*"; }

step "share demo $VERSION"
share_output=$("${KNIFE[@]}" share demo Other "${CONFIG[@]}" -o /work/cookbooks -d -y 2>&1)
printf '%s\n' "$share_output"
grep -q 'Upload complete' <<<"$share_output"

step "download demo $VERSION"
rm -f "$ARCHIVE"
"${KNIFE[@]}" download demo "$VERSION" -f "$ARCHIVE" "${CONFIG[@]}" -d
test -s "$ARCHIVE"
tar -tzf "$ARCHIVE" | grep -q 'metadata.json'

step "show demo $VERSION"
show_output=$("${KNIFE[@]}" show demo "$VERSION" "${CONFIG[@]}")
printf '%s\n' "$show_output"
grep -q 'demo' <<<"$show_output"

step "list"
list_output=$("${KNIFE[@]}" list "${CONFIG[@]}")
printf '%s\n' "$list_output"
grep -q 'demo' <<<"$list_output"

printf '\ndemo: PASS\n'
