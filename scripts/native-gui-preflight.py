"""Read-only console guard before allocating native GUI acceptance resources."""
import subprocess


def require_unlocked_console():
    registry = subprocess.check_output(
        ['/usr/sbin/ioreg', '-l', '-n', 'Root', '-d', '1'], text=True, timeout=5)
    if '"IOConsoleLocked" = No' not in registry:
        raise RuntimeError('Unlock the Mac before native GUI acceptance; no native build or fixture generated')
