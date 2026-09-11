#!/bin/sh
# Keep the documented shell entry point; the gate owns its cwd and outputs.
set -eu
exec python3 -B "$(dirname "$0")/check.py" "$@"
