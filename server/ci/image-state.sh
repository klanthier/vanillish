#!/usr/bin/env bash
# image-state.sh REF INPUTS — what the registry holds at REF, against this checkout's INPUTS
# (pins.sh's vmangos_inputs or datahost_inputs):
#
#   absent  no such tag, or none this client may read (a private package, pulled anonymously)
#   fresh   linux/amd64 and linux/arm64 both there, each labelled io.github.klanthier.vanillish.inputs=INPUTS
#   stale   the tag exists, built from other files (or missing a platform)
#
# Anything else (the registry down, a network error) fails rather than guess.
set -euo pipefail
[ $# = 2 ] || {
    echo "usage: $0 REF INPUTS" >&2
    exit 2
}
ref="$1" inputs="$2"

if ! out="$(docker buildx imagetools inspect "$ref" --format '{{json .Image}}' 2>&1)"; then
    # GHCR: "not found" for a missing tag; "403 Forbidden" for a package anonymous may not see.
    case "$(printf '%s' "$out" | tr '[:upper:]' '[:lower:]')" in
        *"not found"* | *"manifest unknown"* | *"name unknown"* | *denied* | *unauthorized* | *forbidden*)
            echo absent
            exit 0
            ;;
    esac
    echo "image-state.sh: $ref: $out" >&2
    exit 1
fi

# One platform's config is an object with "architecture"; a multi-platform tag maps "os/arch" to one.
printf '%s' "$out" | INPUTS="$inputs" python3 -c '
import json, os, sys
doc = json.load(sys.stdin)
configs = {doc.get("os", "?") + "/" + doc["architecture"]: doc} if "architecture" in doc else doc
label = "io.github.klanthier.vanillish.inputs"
seen = {p: (((c or {}).get("config") or {}).get("Labels") or {}).get(label) for p, c in configs.items()}
for p, value in sorted(seen.items()):
    print("image-state.sh: %s: inputs %s" % (p, value), file=sys.stderr)
want = os.environ["INPUTS"]
print("fresh" if all(seen.get(p) == want for p in ("linux/amd64", "linux/arm64")) else "stale")
'
