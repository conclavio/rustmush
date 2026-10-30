#!/usr/bin/env bash
# The unit bake, one command (doc 22 D-22-10, cut U.6): for each look, render
# its bake matrix in Unity's batch mode from its config
# (scripts/unit-bake/looks/<look>.json) and reduce it into the unit pack
# (crates/mush-pixel/looks/units.bin), printing the pack's size, its digest
# and the matrix check.
#
#   UNITY_PACKAGES=/c/Users/you/3d_assets scripts/unit-bake/bake.sh <unity-project> [look ...]
#
# <unity-project> is a scratch Unity project OUTSIDE the repo (created if it
# does not exist); UNITY_PACKAGES is the folder the configs' `package` paths
# are relative to (each look's .unitypackage is imported once, when its root
# folder is not in the project yet). With no looks named, every look is
# baked. UNITY overrides the editor's path; UNIT_PACK the pack written (a
# trial bake into a scratch pack leaves the tree's alone).
#
# Never commit the packages, the Unity project or the renders (the licensing
# ruling of 2026-10-29): only the configs, this script, Bake.cs and the
# reduced pack ship. The pack records the asset pack, its version (the
# config's, else the package file's SHA-256), the prefab and clips, and the
# bake and reduce commits.
set -euo pipefail

UNITY="${UNITY:-/c/Program Files/Unity/Hub/Editor/6000.6.7f1/Editor/Unity.exe}"
ROOT="$(git rev-parse --show-toplevel)"
HERE="$ROOT/scripts/unit-bake"
PACK="${UNIT_PACK:-$ROOT/crates/mush-pixel/looks/units.bin}"

if [ $# -lt 1 ]; then
  sed -n '2,20p' "$0"
  exit 2
fi
PROJ="$1"
shift
case "$(cd "$(dirname "$PROJ")" 2>/dev/null && pwd)/" in
  "$ROOT"/*) echo "bake.sh: the Unity project must be outside the repo ($PROJ)" >&2; exit 2 ;;
esac
LOOKS=("$@")
if [ ${#LOOKS[@]} -eq 0 ]; then
  LOOKS=(biped tracked_tank wheeled_scout drone heavy unknown)
fi

COMMIT="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
if ! git -C "$ROOT" diff --quiet HEAD -- scripts/unit-bake crates/mush-pixel/src/bake crates/mush-pixel/src/bake.rs crates/mush-pixel/examples/unit_reduce.rs; then
  echo "bake.sh: the bake scripts or the reducer differ from $COMMIT; the provenance will say $COMMIT-dirty" >&2
  COMMIT="$COMMIT-dirty"
fi

field() { python -c "import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2], '') or '')" "$1" "$2"; }

unity() {
  local log="$1"
  shift
  if ! "$UNITY" -batchmode -quit -logFile "$log" "$@"; then
    grep -E "BAKE FAILED|error CS|Exception" "$log" | head -20 >&2 || true
    echo "bake.sh: Unity failed; its log is $log" >&2
    return 1
  fi
}

if [ ! -d "$PROJ/Assets" ]; then
  echo "== creating the Unity project $PROJ"
  unity "$PROJ.create.log" -createProject "$PROJ"
fi
mkdir -p "$PROJ/Assets/Editor" "$PROJ/Assets/BakeShaders" "$PROJ/bake_out"
cp "$HERE/unity/Editor/Bake.cs" "$PROJ/Assets/Editor/"
cp "$HERE"/unity/BakeShaders/*.shader "$PROJ/Assets/BakeShaders/"

for look in "${LOOKS[@]}"; do
  cfg="$HERE/looks/$look.json"
  [ -f "$cfg" ] || { echo "bake.sh: no config $cfg" >&2; exit 2; }
  package="$(field "$cfg" package)"
  root="$(field "$cfg" root)"
  version="$(field "$cfg" asset_version)"
  if [ -n "$package" ]; then
    : "${UNITY_PACKAGES:?set UNITY_PACKAGES to the folder holding $package}"
    file="$UNITY_PACKAGES/$package"
    [ -f "$file" ] || { echo "bake.sh: no package $file" >&2; exit 2; }
    if [ -z "$version" ]; then
      version="sha256:$(sha256sum "$file" | cut -c1-16)"
    fi
    if [ ! -d "$PROJ/${root%/}" ]; then
      echo "== importing $package"
      unity "$PROJ/import_$look.log" -projectPath "$PROJ" -importPackage "$file"
    fi
  fi
  out="$PROJ/bake_out/$look"
  rm -rf "$out"
  echo "== baking $look"
  unity "$PROJ/bake_$look.log" -projectPath "$PROJ" -executeMethod Bake.Run \
    -bakeConfig "$cfg" -bakeOut "$out" -bakeCommit "$COMMIT" -bakeAssetVersion "$version"
  echo "== reducing $look"
  (cd "$ROOT" && cargo run -q --release -j 8 -p mush-pixel --example unit_reduce -- \
    "$out" --pack "$PACK" --reduce-commit "$COMMIT" --preview "$PROJ/bake_out/preview_$look.png")
done

echo "== done: $PACK; pin the digest the reducer printed in crates/mush-pixel/src/bake.rs (PACK_DIGEST),"
echo "   then render the line-ups: cargo run -p mush-pixel --example unit_lineup -- docs/design/images/22-u6"
