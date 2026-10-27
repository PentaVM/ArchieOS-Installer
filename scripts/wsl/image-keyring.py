"""Remove builder keyrings from staging roots; reject them at packaging time."""
import argparse
from pathlib import Path
import shutil


def keyring_path(root):
    root = Path(root).resolve(strict=True)
    if root.parent == root or not (root / 'etc/os-release').is_file():
        raise ValueError('expected a separate staging rootfs, never the live root')
    target = root
    for part in ('etc', 'pacman.d', 'gnupg'):
        target = target / part
        if target.is_symlink() or (hasattr(target, 'is_junction') and target.is_junction()):
            raise ValueError('keyring path contains a symlink/junction')
        if not target.resolve().is_relative_to(root):
            raise ValueError('keyring path escapes staging rootfs')
    return target


def check_image(root):
    target = keyring_path(root)
    if target.exists() and (not target.is_dir() or any(target.iterdir())):
        raise ValueError('builder keyring found; prepare the image before packaging')


def prepare_image(root):
    target = keyring_path(root)
    if target.exists():
        # keyring_path has verified the resolved target stays inside the staging root.
        shutil.rmtree(target)
    target.mkdir(parents=True, mode=0o700)
    check_image(root)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('prepare', 'check'))
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    try:
        (prepare_image if args.operation == 'prepare' else check_image)(args.root)
    except (OSError, ValueError) as error:
        parser.exit(1, f'image-keyring: {error}\n')
