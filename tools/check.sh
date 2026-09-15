#!/bin/sh
# Stable shell entry point: forward arguments and exit status to check.py.
# The Python runner owns source freezing, stage order, processes and outputs.

set -eu
exec python3 -B "$(dirname "$0")/check.py" "$@"
