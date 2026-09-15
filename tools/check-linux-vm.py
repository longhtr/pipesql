#!/usr/bin/env python3
"""Run the existing Linux gate on a full-synchronization Apple virtual disk.

Docker only prepares an already provisioned toolchain image and reads quiescent
results. Database operations run inside the separate, offline Linux VM.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shlex
import shutil
import subprocess
import sys
import uuid

from check_process import run
from check import stages

ROOT = Path(__file__).resolve().parent.parent


def checked(command, *, timeout=180, **options):
    return run(list(map(str, command)), cwd=ROOT, timeout=timeout, check=True, **options)


def prepare_guest():
    """Inside the preparation container, populate a read-only ext4 boot image."""
    output = Path('/input')
    boot = Path('/tmp/boot-root')
    boot.mkdir()
    checked(['tar', '--no-same-owner', '-xf', output / 'rootfs.tar', '-C', boot])
    shutil.copytree(ROOT, boot / 'source')
    checked(['cc', '-static', '-O2', '-Wall', '-Wextra', '-Wconversion', '-Werror',
             ROOT / 'tools/fixtures/linux-verification-init.c', '-o', boot / 'init'])
    shutil.copy2(ROOT / 'tools/fixtures/linux-verification.sh', boot / 'verification.sh')
    environment = ('PATH', 'CARGO_HOME', 'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN')
    (boot / 'verification-env.sh').write_text(''.join(
        f'export {name}={shlex.quote(os.environ[name])}\n'
        for name in environment if name in os.environ
    ))
    checked(['mkfs.ext4', '-q', '-F', '-d', boot, output / 'boot.raw', '1572864'])
    checked(['mkfs.ext4', '-q', '-F', output / 'empty.raw', '8388608'])


def guest_result(text):
    """VM shutdown alone is insufficient: require status and completed cleanup."""
    lines = text.splitlines()
    statuses = [line for line in lines if line.startswith('VERIFICATION_RESULT ')]
    if len(statuses) != 1 or lines.count('VERIFICATION_STOPPED') != 1:
        raise ValueError('missing or duplicate guest completion')
    if any(line.startswith('VERIFICATION_ERROR ') for line in lines):
        raise ValueError('guest bootstrap or cleanup failed')
    match = re.fullmatch(r'VERIFICATION_RESULT exit=(\d+)', statuses[0])
    if match is None:
        raise ValueError('malformed guest exit status')
    if lines.index(statuses[0]) > lines.index('VERIFICATION_STOPPED'):
        raise ValueError('guest status follows cleanup completion')
    status = int(match[1])
    if status > 255:
        raise ValueError('invalid guest exit status')
    return status


def validate_gate(gate, source_sha256):
    if gate is None or gate.get('status') != 'passed' or gate.get('scope') != 'full':
        raise ValueError('full guest gate did not pass')
    if gate.get('source_sha256') != source_sha256 or gate.get('inputs_unchanged') is not True:
        raise ValueError('guest source differs from the frozen export')
    if gate.get('finalization_errors') != []:
        raise ValueError('guest gate cleanup failed')
    required = [stage.name for stage in stages('full', Path('/gate'))]
    observed = gate.get('stages', [])
    if [stage.get('name') for stage in observed] != required or any(
        stage.get('status') != 'passed' or stage.get('returncode') != 0
        for stage in observed
    ):
        raise ValueError('guest gate has missing, reordered or failed stages')


source_export = runpy.run_path(str(ROOT / 'tools/source-manifest.py'))['source_export']

def execute(image, kernel, output, bootstrap_only):
    work = output / 'work'
    work.mkdir()
    containers = []
    receipt = {'status': 'failed', 'scope': 'bootstrap' if bootstrap_only else 'full',
               'disk_policy': 'full', 'cpus': 1, 'memory_bytes': 2147483648,
               'network_devices': 0, 'guest_uid': 1000, 'guest_gid': 1000}
    try:
        identity = checked(['docker', 'image', 'inspect', image], capture_output=True, text=True)
        inspected = json.loads(identity.stdout)[0]
        if inspected['Architecture'] != 'arm64' or inspected['Os'] != 'linux':
            raise ValueError('image must be GNU arm64 Linux with the pinned tools')
        receipt['image'] = inspected['Id']
        receipt['kernel_sha256'] = hashlib.sha256(kernel.read_bytes()).hexdigest()
        manifest = source_export(ROOT, work / 'source')
        (output / 'inputs.sha256').write_text(manifest)
        receipt['source_sha256'] = hashlib.sha256(manifest.encode()).hexdigest()
        tag = uuid.uuid4().hex[:12]
        base, builder = f'pipesql-vm-base-{tag}', f'pipesql-vm-build-{tag}'
        containers.append(base)
        checked(['docker', 'create', '--name', base, '--network', 'none', image])
        checked(['docker', 'export', base, '-o', work / 'rootfs.tar'])
        containers.append(builder)
        checked(['docker', 'run', '-d', '--name', builder, '--user', '1000:1000',
                 '--cpus', '1', '--memory', '2g', '--memory-swap', '2g', '--network', 'none',
                 '--mount', f'type=bind,source={work},target=/input', image, 'sleep', 'infinity'])
        checked(['docker', 'exec', builder, 'python3', '-B', '-c',
                 "import sys, runpy;sys.path.insert(0, '/input/source/tools');"
                 "runpy.run_path('/input/source/tools/check-linux-vm.py')['prepare_guest']()"],
                timeout=600)
        controller = work / 'host'
        checked(['cc', '-fobjc-arc', '-Wall', '-Wextra', '-Werror', '-framework', 'Foundation',
                 '-framework', 'Virtualization',
                 work / 'source/tools/fixtures/virtual-disk-sync-host.m', '-o', controller])
        entitlement = work / 'entitlements.plist'
        entitlement.write_text('<plist version="1.0"><dict>'
                               '<key>com.apple.security.virtualization</key><true/></dict></plist>')
        checked(['codesign', '--force', '--sign', '-', '--entitlements', entitlement, controller])
        receipt['controller_sha256'] = hashlib.sha256(controller.read_bytes()).hexdigest()
        # Reject a weaker profile before any VM or database operation starts.
        refused = run([str(controller), str(kernel), str(work / 'boot.raw'),
                       str(work / 'empty.raw'), 'fsync', 'verify'], cwd=ROOT,
                      timeout=30, capture_output=True, text=True)
        if refused.returncode != 2 or 'verification requires full' not in refused.stderr:
            raise ValueError('controller did not reject weaker synchronization')
        modes = ['verify-smoke', 'verify-fail'] + ([] if bootstrap_only else ['verify'])
        for mode in modes:
            disk = work / f'{mode}.raw'
            # APFS clones keep the 8-GiB sparse initial images cheap and identical.
            checked(['cp', '-c', work / 'empty.raw', disk])
            log = output / f'{mode}.log'
            with log.open('x') as stream:
                checked([controller, kernel, work / 'boot.raw', disk, 'full', mode],
                        timeout=7250, stdout=stream, stderr=subprocess.STDOUT)
            text = log.read_text()
            if 'CPU=1 memory=2147483648 cache=2 sync=1 network=0' not in text:
                raise ValueError('VM configuration record disagrees')
            gate = None
            if mode == 'verify' and 'gate scope=full output=/tmp/gate' in text:
                # Copy failure receipts too, before accepting the guest status.
                # The controller has exited and the guest disk is quiescent.
                checked(['docker', 'exec', builder, 'mkdir', '/tmp/results'])
                checked(['docker', 'exec', builder, 'debugfs', '-R',
                         'rdump /gate /tmp/results', f'/input/{disk.name}'])
                checked(['docker', 'cp', f'{builder}:/tmp/results/gate', output / 'gate'])
                gate = json.loads((output / 'gate/result.json').read_text())
            result = guest_result(text)
            expected = 23 if mode == 'verify-fail' else 0
            if result != expected:
                raise ValueError(f'{mode} returned {result}, expected {expected}; see {log}')
            if mode == 'verify':
                validate_gate(gate, receipt['source_sha256'])
                if 'fresh examples passed: 4' not in text:
                    raise ValueError('fresh examples did not finish')
            print(f'{mode} passed (guest exit {result})', flush=True)
            disk.unlink()
        receipt['status'] = 'passed'
    except BaseException as error:
        receipt['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        failures = []
        for container in reversed(containers):
            try:
                checked(['docker', 'rm', '-f', container])
            except Exception as error:
                failures.append(str(error))
        try:
            shutil.rmtree(work)
        except Exception as error:
            failures.append(str(error))
        receipt['cleanup_errors'] = failures
        if failures:
            receipt['status'] = 'failed'
        (output / 'environment.json').write_text(json.dumps(receipt, indent=2) + '\n')
        if failures:
            raise RuntimeError('; '.join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True, help='already provisioned GNU arm64 toolchain image')
    parser.add_argument('--kernel', required=True, type=Path, help='local arm64 Linux kernel')
    parser.add_argument('--output', required=True, type=Path, help='new result directory outside checkout')
    parser.add_argument('--bootstrap-only', action='store_true', help='controls only; not a full gate')
    options = parser.parse_args()
    if sys.platform != 'darwin' or os.uname().machine != 'arm64' or not __debug__:
        parser.error('requires Apple silicon macOS and Python assertions')
    output = options.output.absolute()
    if output.is_symlink() or output == ROOT or ROOT in output.resolve().parents:
        parser.error('output must be a new directory outside the checkout')
    kernel = options.kernel.resolve(strict=True)
    output.mkdir()
    execute(options.image, kernel, output.resolve(), options.bootstrap_only)


if __name__ == '__main__':
    main()
