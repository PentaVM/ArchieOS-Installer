import argparse
import json
from .contracts import BASE_SHA
from .model import inspect_model


def main():
    parser = argparse.ArgumentParser(description='Exact-checkpoint Wanda worker; no Core authority')
    commands = parser.add_subparsers(dest='command', required=True)
    inspect = commands.add_parser('inspect')
    inspect.add_argument('--model', required=True)
    extract = commands.add_parser('verify-extraction')
    extract.add_argument('--config', required=True)
    extract.add_argument('--headers', required=True)
    extract.add_argument('--cases', required=True)
    extract.add_argument('--out', required=True)
    for command, gate in [('verify-suffix','extraction'),('verify-loss','parity')]:
        verify=commands.add_parser(command)
        verify.add_argument('--config',required=True)
        verify.add_argument('--'+gate,required=True)
        verify.add_argument('--out',required=True)
    args = parser.parse_args()
    if args.command == 'inspect':
        model = inspect_model(args.model, BASE_SHA)
        print(json.dumps({'format_version': 1, 'base_sha256': model.sha256, 'bytes': model.size,
                          'metadata_sha256': model.metadata_sha256, 'tensors': len(model.tensors),
                          'architecture': model.fields.get('general.architecture')}))
    elif args.command == 'verify-extraction':
        from .contracts import TrainConfig
        from .native import verify_extraction
        result=verify_extraction(TrainConfig.load(args.config),args.headers,args.cases,args.out)
        print(json.dumps(result))
    elif args.command in ('verify-suffix','verify-loss'):
        from .contracts import TrainConfig
        from .verification import verify_suffix, verify_loss
        config=TrainConfig.load(args.config)
        result=(verify_suffix(config,args.extraction,args.out) if args.command=='verify-suffix'
                else verify_loss(config,args.parity,args.out))
        print(json.dumps(result))


if __name__ == '__main__':
    main()
