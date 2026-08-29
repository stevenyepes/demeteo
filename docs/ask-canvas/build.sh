#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
for n in "$@"; do
  {
    cat _head_a.html
    cat _tokens.css
    [ -f "css/$n.css" ] && cat "css/$n.css"
    cat _head_b.html
    cat "body/$n.html"
    cat _tail.html
  } > "$n.dc.html"
  echo "built $n.dc.html ($(wc -c < "$n.dc.html") bytes)"
done
