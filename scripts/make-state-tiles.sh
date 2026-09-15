#!/usr/bin/env bash
# Renders the README's state table from the one silhouette the whole product
# draws. It picks whichever SVG renderer the machine has: rsvg-convert,
# ImageMagick, or, on a stock Mac, sips. Run it whenever a state is added or
# the mark changes, then commit what it writes.
#
# The menu bar images in bansheed/assets/tray are not written here. Those are
# template images macOS tints, drawn at 36px where the arcs need their own
# spacing, and they are the artist's originals.
#
# Each renderer produces different antialiasing in the tiles. A regeneration on a
# different platform rewrites some of them. This is not a drawing change.
# Measured error: RMSE 0.0025 to 0.0130 on a 0 to 1 scale.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/assets/states"

# Copied from assets/banshee-mark.svg, which Mark.svelte also copies.
shroud='M21 70 L24 46 C24 27 34 14 50 14 C66 14 76 27 76 46 L79 70 C79 80 72 88 64 86 C57 84 55 72 50 72 C45 72 43 84 36 86 C28 88 21 80 21 70 Z'

# The halo busy draws: two concentric flat ellipses, the inner one cut out by
# fill-rule evenodd. Copied from Mark.svelte's RING.
halo='M6 40 a44 18 0 1 0 88 0 a44 18 0 1 0 -88 0 z M9.5 38.8 a40 14.2 0 1 0 80 0 a40 14.2 0 1 0 -80 0 z'

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
        # Over-ear cups, clear of the silhouette, so the form never reads as the
        # filled shroud recording uses.
        listening)
            printf '<ellipse cx="17" cy="46" rx="12" ry="17" fill="%s"/>' "$ink"
            printf '<ellipse cx="83" cy="46" rx="12" ry="17" fill="%s"/>' "$ink"
            ;;
        # The halo passes behind the head. The mask hides it wherever the shroud
        # is, and the second path draws the near half back over the front.
        busy)
            printf '<mask id="behind"><rect x="-40" y="-40" width="180" height="180" fill="#fff"/>'
            printf '<path d="%s" fill="#000" stroke="#000" stroke-width="15" stroke-linejoin="round"/></mask>' "$shroud"
            printf '<clipPath id="near"><rect x="-40" y="40" width="180" height="100"/></clipPath>'
            printf '<path d="%s" fill="%s" fill-rule="evenodd" mask="url(#behind)"/>' "$halo" "$ink"
            printf '<path d="%s" fill="%s" fill-rule="evenodd" clip-path="url(#near)"/>' "$halo" "$ink"
            ;;
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

# Ordered by fidelity: rsvg-convert, then ImageMagick under either name, then
# sips for a stock Mac with nothing installed. Each is asked for the tile size
# exactly, so the renderer never changes the output's size or shape.
if command -v rsvg-convert >/dev/null 2>&1; then
    render() { rsvg-convert --width "$tile" --height "$tile" "$1" --output "$2"; }
elif command -v magick >/dev/null 2>&1; then
    render() { magick -background none "$1" -resize "${tile}x${tile}" "$2"; }
elif command -v convert >/dev/null 2>&1; then
    render() { convert -background none "$1" -resize "${tile}x${tile}" "$2"; }
elif command -v sips >/dev/null 2>&1; then
    render() { sips -s format png "$1" --out "$2" >/dev/null; }
else
    echo "make-state-tiles.sh needs an SVG renderer. Install one:" >&2
    echo "  rsvg-convert - apt install librsvg2-bin, or brew install librsvg" >&2
    echo "  ImageMagick  - apt install imagemagick, or brew install imagemagick" >&2
    echo "  sips         - already on macOS, nothing to install" >&2
    exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out"

for state in idle recording busy speaking listening notrunning; do
    {
        printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %s %s" width="%s" height="%s">' \
            "$tile" "$tile" "$tile" "$tile"
        printf '<rect width="%s" height="%s" rx="%s" fill="%s"/>' "$tile" "$tile" "$radius" "$ground"
        printf '<g transform="translate(%s,%s) scale(%s)">' "$inset" "$inset" "$scale"
        body "$state"
        decoration "$state"
        printf '</g></svg>'
    } > "$work/$state.svg"
    render "$work/$state.svg" "$out/$state.png"
    echo "wrote $out/$state.png"
done
