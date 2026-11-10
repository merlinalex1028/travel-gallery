"""Count source inputs separately from exports in an owned native fixture."""
from pathlib import Path


def source_inventory(fixture: Path):
    if fixture.is_symlink():
        raise RuntimeError('Native fixture root must not be a symbolic link')
    files = []
    for name in ('source', 'held-out'):
        root = fixture/name
        if root.is_symlink():
            raise RuntimeError('Native source root must not be a symbolic link')
        for path in sorted(root.rglob('*')):
            if path.is_symlink():
                raise RuntimeError('Native source inventory must not follow symbolic links')
            if path.is_file():
                files.append(path)
    return files
