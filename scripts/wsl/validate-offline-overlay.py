"""Reject legacy launchers accidentally inherited from the clean base image."""
from pathlib import Path
import sys

def verify_overlay(overlay, root):
    for source in (overlay / 'usr').rglob('*'):
        if not source.is_file() or '__pycache__' in source.parts:
            continue
        relative = source.relative_to(overlay)
        target = root / relative
        if not target.is_file() or source.read_bytes() != target.read_bytes():
            raise ValueError('stale/missing source overlay: ' + str(relative))

if __name__ == '__main__':
    verify_overlay(Path(sys.argv[1]), Path(sys.argv[2]))
    print('Current source overlay matches the image.')
