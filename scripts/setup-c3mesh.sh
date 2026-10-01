#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dependency_path="$repository_root/../c3mesh"
revision=$(tr -d '\r\n' < "$repository_root/c3mesh-revision.txt")
case "$revision" in
    *[!0-9a-f]*|'') echo "Invalid c3mesh revision." >&2; exit 1 ;;
esac
[ "${#revision}" -eq 40 ] || { echo "Expected a complete c3mesh commit SHA." >&2; exit 1; }
verify_only=false
case "${1:-}" in
    --verify-only) verify_only=true ;;
    '') ;;
    *) echo "Usage: $0 [--verify-only]" >&2; exit 1 ;;
esac

if [ ! -e "$dependency_path" ]; then
    if "$verify_only"; then echo "Missing c3mesh checkout." >&2; exit 1; fi
    git clone https://github.com/dancarpenter21/c3mesh.git "$dependency_path"
fi
dependency_path=$(CDPATH= cd -- "$dependency_path" && pwd)
repository_top=$(git -C "$dependency_path" rev-parse --show-toplevel)
repository_top=$(CDPATH= cd -- "$repository_top" && pwd)
[ "$repository_top" = "$dependency_path" ] || { echo "Expected c3mesh repository root." >&2; exit 1; }
current_revision=$(git -C "$dependency_path" rev-parse --verify HEAD)
if "$verify_only"; then
    [ "$current_revision" = "$revision" ] || { echo "c3mesh differs from pinned $revision." >&2; exit 1; }
    [ -z "$(git -C "$dependency_path" status --porcelain)" ] || { echo "c3mesh has local changes." >&2; exit 1; }
elif [ "$current_revision" != "$revision" ]; then
    [ -z "$(git -C "$dependency_path" status --porcelain)" ] || {
        echo "c3mesh has local changes; commit or stash them before setup." >&2; exit 1;
    }
    git -C "$dependency_path" fetch origin "$revision"
    git -C "$dependency_path" checkout --detach "$revision"
fi
echo "c3mesh $revision is available at $dependency_path"
