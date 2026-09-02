#!/usr/bin/env bash
# Renders the README's state table from the one silhouette the whole product
# draws, with macOS built-ins only. Run it whenever a state is added or the
# mark changes, then commit what it writes.
#
# The menu bar images in bansheed/assets/tray are not written here. Those are
# template images macOS tints, drawn at 36px where the arcs need their own
# spacing, and they are the artist's originals.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/assets/states"

# Copied from assets/banshee-mark.svg, which Mark.svelte also copies.
shroud='M21 70 L24 46 C24 27 34 14 50 14 C66 14 76 27 76 46 L79 70 C79 80 72 88 64 86 C57 84 55 72 50 72 C45 72 43 84 36 86 C28 88 21 80 21 70 Z'

# The frame and the two colours of assets/banshee-icon.svg, so the row sits
# under the same identity as the icon at the top of the README. Monochrome on
# purpose: the menu bar draws a template image, so shape alone separates the
# states. The figure is never the accent, which means recording in the window.
tile=120
radius=27
ground='#8A2A0D'
ink='#F2EFE9'
# The row renders at 52px, so the figure takes more of the tile than the app
# icon gives it and still reads at that size.
inset=21
scale=0.78

decoration() {
    case "$1" in
        # It must not read as recording, the only other form with solid ink.
        listening) printf '<rect x="33" y="52" width="34" height="10" fill="%s"/>' "$ink" ;;
        # The gap is the point: closed up, the arcs read as earmuffs rather than
        # as sound leaving the figure.
        speaking)
            for d in 'M8 40 C4 48 4 56 8 64' 'M92 40 C96 48 96 56 92 64'; do
                printf '<path d="%s" fill="none" stroke="%s" stroke-width="6" stroke-linecap="round"/>' "$d" "$ink"
            done
            ;;
    esac
}

body() {
    case "$1" in
        recording) printf '<path d="%s" fill="%s" stroke="%s" stroke-width="9" stroke-linejoin="round"/>' "$shroud" "$ink" "$ink" ;;
        notrunning) printf '<path d="%s" fill="none" stroke="%s" stroke-width="9" stroke-linejoin="round" stroke-dasharray="22 14"/>' "$shroud" "$ink" ;;
        *) printf '<path d="%s" fill="none" stroke="%s" stroke-width="9" stroke-linejoin="round"/>' "$shroud" "$ink" ;;
    esac
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out"

for state in idle recording speaking listening notrunning; do
    {
        printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %s %s" width="%s" height="%s">' \
            "$tile" "$tile" "$tile" "$tile"
        printf '<rect width="%s" height="%s" rx="%s" fill="%s"/>' "$tile" "$tile" "$radius" "$ground"
        printf '<g transform="translate(%s,%s) scale(%s)">' "$inset" "$inset" "$scale"
        body "$state"
        decoration "$state"
        printf '</g></svg>'
    } > "$work/$state.svg"
    sips -s format png "$work/$state.svg" --out "$out/$state.png" >/dev/null
    echo "wrote $out/$state.png"
done
