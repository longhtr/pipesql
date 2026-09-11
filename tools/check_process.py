"""Owned subprocesses for native verification on macOS and Linux.

Each launch creates a process group. The caller owns that group until this scope
ends, including children left behind by a timing wrapper or compiler. Campaigns
must not detach descendants into another session. Windows needs a job-object
implementation before these native campaigns can use the same cleanup contract.
"""

from contextlib import contextmanager
import math
import os
from pathlib import Path
import signal
import subprocess
import sys


def _signal_group(child, signum):
    try:
        os.killpg(child.pid, signum)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Darwin's killpg1 skips zombies and can report EPERM for a group with
        # no live members. Do not mistake that state for a failed cleanup, or
        # suppress a real permission failure affecting a live process.
        if sys.platform != "darwin":
            raise
        members = subprocess.run(
            ["/bin/ps", "-axo", "pgid=,stat="],
            cwd="/",
            capture_output=True,
            text=True,
            check=True,
            timeout=5,
        )
        for row in members.stdout.splitlines():
            group, state = row.split()
            if int(group) == child.pid and not state.startswith("Z"):
                raise


def _close_group(child, terminate_timeout):
    # Let nested checkers release their own process groups before escalation.
    # A successful leader can also leave descendants, so always finish the group.
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=terminate_timeout)
        except subprocess.TimeoutExpired:
            pass
    _signal_group(child, signal.SIGKILL)
    child.wait(timeout=5)


@contextmanager
def owned_process(command, *, cwd, terminate_timeout=2, **options):
    """Launch from the main thread; close the group on return or interruption.

    The caller must bound any pipe reads and waits within the scope. SIGTERM becomes
    SystemExit so normal Python unwinding releases the process and campaign output.
    SIGKILL cannot run cleanup. Commands are argument lists, never shell programs.
    """
    if os.name != "posix":
        raise NotImplementedError("native campaign process ownership requires POSIX")
    if not Path(cwd).is_absolute():
        raise ValueError("supply an absolute working directory")
    if not math.isfinite(terminate_timeout) or terminate_timeout < 0:
        raise ValueError("termination timeout must be finite and nonnegative")
    if isinstance(command, (str, bytes)) or "shell" in options:
        raise ValueError("supply a command argument list, without shell execution")
    if "start_new_session" in options or "process_group" in options:
        raise ValueError("the campaign owns the child's process group")

    def terminate(signum, frame):
        raise SystemExit(128 + signum)

    previous = signal.signal(signal.SIGTERM, terminate)
    child = None
    try:
        child = subprocess.Popen(command, cwd=cwd, start_new_session=True, **options)
        yield child
    finally:
        try:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            if child is not None:
                try:
                    _close_group(child, terminate_timeout)
                finally:
                    for stream in (child.stdin, child.stdout, child.stderr):
                        if stream is not None:
                            stream.close()
        finally:
            signal.signal(signal.SIGTERM, previous)


def run(command, *, cwd, timeout, check=False, capture_output=False, **options):
    """Run an owned command with a deadline and subprocess-compatible results."""
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    if capture_output:
        if "stdout" in options or "stderr" in options:
            raise ValueError("capture_output cannot be combined with stdout or stderr")
        options.update(stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    with owned_process(command, cwd=cwd, **options) as child:
        stdout, stderr = child.communicate(timeout=timeout)
        result = subprocess.CompletedProcess(command, child.returncode, stdout, stderr)
        if check:
            result.check_returncode()
    return result
