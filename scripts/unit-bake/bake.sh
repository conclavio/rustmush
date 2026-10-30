#!/usr/bin/env bash
# The unit bake, one command (doc 22 D-22-10, cut U.6; doc 31 cut UD.1): for each model, render
# its bake matrix in Unity's batch mode from its config
# (scripts/unit-bake/models/<model>.json) and reduce it into the model catalogue
# (crates/mush-pixel/models/: its content-named file and index.json), printing the file's size, its SHA-256
# and the matrix check.
#
#   UNITY_PACKAGES=/c/Users/you/3d_assets scripts/unit-bake/bake.sh <unity-project> [model ...]
#
# <unity-project> is a scratch Unity project OUTSIDE the repo (created if it
# does not exist); UNITY_PACKAGES is the folder the configs' `package` paths
# are relative to (each model's .unitypackage is imported once, when its root
# folder is not in the project yet). With no models named, every model with a
# config is baked. UNITY overrides the editor's path; UNIT_MODELS the catalogue
# written (a trial bake into a scratch folder leaves the tree's alone).
#
# Never commit the packages, the Unity project or the renders (the licensing
# ruling of 2026-10-29): only the configs, this script, Bake.cs and the
# reduced model files ship. Each file records the asset pack, its version (the
# config's, else the package file's SHA-256), the prefab and clips, and the
# bake and reduce commits.
set -euo pipefail

UNITY="${UNITY:-/c/Program Files/Unity/Hub/Editor/6000.6.7f1/Editor/Unity.exe}"
ROOT="$(git rev-parse --show-toplevel)"
HERE="$ROOT/scripts/unit-bake"
MODELS="${UNIT_MODELS:-$ROOT/crates/mush-pixel/models}"

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
  LOOKS=()
  for c in "$HERE"/models/*.json; do LOOKS+=("$(basename "$c" .json)"); done
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
  cfg="$HERE/models/$look.json"
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
    "$out" --models "$MODELS" --reduce-commit "$COMMIT" --preview "$PROJ/bake_out/preview_$look.png")
done

echo "== done: $MODELS (each model's file is named by its SHA-256; the catalogue test checks it),"
echo "   then render the line-ups: cargo run -p mush-pixel --example unit_lineup -- docs/design/images/22-u6"
