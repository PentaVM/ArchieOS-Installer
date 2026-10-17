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
    worker = commands.add_parser('worker', help='queue-driven local trainer; runs until --cutoff')
    worker.add_argument('--config', required=True)
    worker.add_argument('--headers', required=True)
    worker.add_argument('--cutoff', required=True, help='ISO-8601 local time with offset')
    status = commands.add_parser('status', help='read-only summary; allocates no GPU memory')
    status.add_argument('--root', required=True)
    gen = commands.add_parser('gen-tom', help='write a procedural ToM chat-v1 tranche (reproducible by seed)')
    gen.add_argument('--count', type=int, required=True)
    gen.add_argument('--seed', type=int, required=True)
    gen.add_argument('--out', required=True)
    gen.add_argument('--version', type=int, default=1, choices=(1, 2),
                     help='2 = minimal counterfactual pairs + think/direct mix')
    gen.add_argument('--direct-share', type=float, default=0.5, help='version 2: share of direct-mode problems')
    rescore = commands.add_parser('bench-rescore', help='recompute a bench report from saved responses (CPU)')
    rescore.add_argument('--dir', required=True)
    rescore.add_argument('--out', required=True)
    export = commands.add_parser('export', help='write an unpromoted native GGUF LoRA candidate')
    export.add_argument('--checkpoint', required=True)
    export.add_argument('--out', required=True)
    native = commands.add_parser('verify-native', help='native LoRA candidate vs trained suffix')
    native.add_argument('--config', required=True)
    native.add_argument('--headers', required=True)
    native.add_argument('--candidate', required=True)
    native.add_argument('--checkpoint', required=True)
    native.add_argument('--cases', required=True)
    native.add_argument('--out', required=True)
    native.add_argument('--pause-root', help='pause the owned worker in this run root while verifying')
    args = parser.parse_args()
    if args.command == 'gen-tom':
        from collections import Counter
        from .chatdata import write_tranche
        from .tomgen import generate, generate_v2
        problems = (generate_v2(args.count, seed=args.seed, direct_share=args.direct_share) if args.version == 2
                    else generate(args.count, seed=args.seed))
        examples = write_tranche(problems, args.out, {'kind': 'owned-procedural', 'generator': 'archie_wanda.tomgen',
                                                      'version': args.version, 'seed': args.seed, 'count': args.count,
                                                      'direct_share': args.direct_share if args.version == 2 else None})
        print(json.dumps({'examples': len(examples), 'splits': Counter(e['split'] for e in examples),
                          'kinds': Counter(e['variant'] for e in examples)}))
        return
    if args.command == 'bench-rescore':
        from .bench import rescore
        from .verification import atomic_json
        result = rescore(args.dir)
        atomic_json(args.out, result)
        print(json.dumps(result['report'], indent=2))
        return
    if args.command == 'export':
        from .export import export_candidate
        print(json.dumps(export_candidate(args.checkpoint, args.out)))
        return
    if args.command == 'verify-native':
        from .contracts import TrainConfig
        from .verification import verify_native
        from .worker_status import paused_worker
        config = TrainConfig.load(args.config)
        with paused_worker(args.pause_root):
            print(json.dumps(verify_native(config, args.headers, args.candidate, args.checkpoint,
                                           args.cases, args.out)))
        return
    if args.command == 'worker':
        from datetime import datetime
        from .contracts import TrainConfig
        from .worker import Worker
        config = TrainConfig.load(args.config)
        cutoff = datetime.fromisoformat(args.cutoff)
        if cutoff.tzinfo is None:
            parser.error('--cutoff needs an explicit UTC offset')
        Worker(config, config.run_dir, args.headers, cutoff.timestamp()).run_forever()
        return
    if args.command == 'status':
        from .worker_status import summarize
        print(json.dumps(summarize(args.root), indent=2, ensure_ascii=False))
        return
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
