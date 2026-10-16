import argparse
import json
from .contracts import BASE_SHA
from .model import inspect_model


def main():
    parser = argparse.ArgumentParser(description='Exact-checkpoint Wanda worker; no Core authority')
    commands = parser.add_subparsers(dest='command', required=True)
    inspect = commands.add_parser('inspect')
    inspect.add_argument('--model', required=True)
    args = parser.parse_args()
    if args.command == 'inspect':
        model = inspect_model(args.model, BASE_SHA)
        print(json.dumps({'format_version': 1, 'base_sha256': model.sha256, 'bytes': model.size,
                          'metadata_sha256': model.metadata_sha256, 'tensors': len(model.tensors),
                          'architecture': model.fields.get('general.architecture')}))


if __name__ == '__main__':
    main()
