#!/usr/bin/env bash
set -Eeuo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# macOS may resolve python3 to Apple's 3.9 even with a newer mise install.
python="${DBX_PYTHON:-}"
if [[ -z $python ]]; then
  for candidate in python3 python3.14 python3.13 python3.12 python3.11; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import tomllib' 2>/dev/null; then
      python=$("$candidate" -c 'import sys; print(sys.executable)')
      break
    fi
  done
fi
if [[ -z $python ]] && command -v mise >/dev/null 2>&1; then
  installed=$(mise where python 2>/dev/null || true)
  if [[ -n $installed && -x $installed/bin/python3 ]]; then python="$installed/bin/python3"; fi
fi
[[ -n $python ]] && "$python" -c 'import tomllib' 2>/dev/null || {
  echo 'Python 3.11+ is required. Install it with mise, or set DBX_PYTHON to its executable.' >&2
  exit 1
}
# Packaging helpers invoke python3 themselves.
export PATH="$(dirname -- "$python"):$PATH"
export PYTHONDONTWRITEBYTECODE=1
exec "$python" "$root/scripts/release-local.py" "$@"
