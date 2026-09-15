#!/bin/sh
# Run the existing gate and four fresh examples after checking the VM premise.
# Init owns mounts and uid/gid; this script verifies private ext4, one CPU and
# no network device. The deliberate failure mode proves status propagation.
# A successful full run removes example outputs before printing completion;
# init and check-linux-vm.py then verify shutdown and the frozen gate receipt.

set -eu
. /verification-env.sh
export CARGO_BUILD_JOBS=1
export PYTHONDONTWRITEBYTECODE=1
export TMPDIR=/tmp
python3 -B - <<'PY'
import os
from pathlib import Path
assert os.getuid() == 1000 and os.getgid() == 1000
assert os.cpu_count() == 1
assert Path('/dev/fd').samefile('/proc/self/fd')
assert list(Path('/dev/fd').iterdir())
interfaces = sorted(Path('/sys/class/net').iterdir())
print('guest interfaces=' + ', '.join(p.name for p in interfaces), flush=True)
assert not any((p / 'device').exists() for p in interfaces), interfaces
mount = next(line for line in Path('/proc/self/mountinfo').read_text().splitlines()
             if line.split()[4] == '/tmp')
assert ' - ext4 /dev/vdb rw' in mount, mount
print('guest uid=1000 gid=1000 cpus=1 network=none data=ext4', flush=True)
print(mount, flush=True)
PY
case "$1" in
    verify-fail) exit 23 ;;
    verify-smoke)
        rustc -vV
        cargo --version
        cc --version
        python3 -B tools/check-filesystem-abi.py
        exit 0 ;;
    verify) ;;
    *) exit 2 ;;
esac
python3 -B tools/check.py --output /tmp/gate
# Exercise complete user workflows on new databases after the gate has cleaned
# its build. The commands and literal/model checks belong to these examples.
export CARGO_TARGET_DIR=/tmp/example-target
cargo build --release --offline --locked --example declared --example event_report --example scaled_report
"$CARGO_TARGET_DIR/release/examples/declared" /tmp/declared
"$CARGO_TARGET_DIR/release/examples/event_report" /tmp/events
"$CARGO_TARGET_DIR/release/examples/scaled_report" /tmp/even-high 32000000 even
"$CARGO_TARGET_DIR/release/examples/scaled_report" /tmp/even-low 8000000 even
rm -rf /tmp/example-target /tmp/declared /tmp/events /tmp/even-high /tmp/even-low
printf 'fresh examples passed: 4\n'
