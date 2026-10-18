"""Long multi-thread group chats for the social-context-pack measurement (evaluation only).

Each world is one long chat (target length in characters ≈ tokens × 3.5) where several
facts are stated and later updated while some people are absent, buried among chatter.
The same questions are asked over two forms of the same world:
- raw: the full transcript;
- pack: the compacted social context pack (header, append-only knowledge ledger with who
  was present for each update, recent window verbatim), see
  _dev/verification/rtx3070/2026-10-17-wanda-social-context-pack-design.md.
Labels come from the simulation, never from a model.
"""
import random

PEOPLE = ['Alice', 'Bruno', 'Carla', 'Diego', 'Elena', 'Farid', 'Greta', 'Hugo']
FACTS = [('the trip', 'is in {v}', ['May', 'June', 'July', 'August', 'September']),
         ('the budget', 'is {v}', ['800 euros', '950 euros', '1,200 euros', '1,500 euros', '2,000 euros']),
         ('the meeting point', 'is {v}', ['the harbour hall', 'the old library', 'the rooftop', 'the gym', 'the station']),
         ('the dinner', 'is on {v}', ['Monday', 'Tuesday', 'Thursday', 'Friday', 'Sunday']),
         ('the deadline', 'is {v}', ['the 3rd', 'the 10th', 'the 15th', 'the 21st', 'the 28th'])]
CHATTER = ['Did anyone watch the match last night?', 'I am stuck in traffic again.', 'That photo is great.',
           'Can someone send me the playlist?', 'My cat knocked over the plant this morning.',
           'Coffee first, then I can think.', 'Who borrowed my charger?', 'The new bakery is really good.',
           'I finally fixed the bike.', 'Rain all week, apparently.', 'Ha, classic.', 'Agreed.',
           'Let me check and come back to you.', 'I will be a bit late today.', 'Nice, thanks!',
           'Has anyone tried the new climbing gym?', 'My phone battery is dying.', 'Sending hugs to everyone.']


def build(seed, target_chars=84000):
    rng = random.Random(seed)
    people = rng.sample(PEOPLE, 6)
    facts = rng.sample(FACTS, 3)
    present = set(people[:4])
    lines, ledger, t = [f'[0] Chat members now: {", ".join(sorted(present))}.'], [], 0
    state = {f[0]: rng.choice(f[2]) for f in facts}
    knowledge = {f[0]: {p: (state[f[0]] if p in present else None) for p in people} for f in facts}
    for topic, verb, _ in facts:
        t += 1
        lines.append(f'[{t}] {rng.choice(sorted(present))}: {topic.capitalize()} {verb.format(v=state[topic])}.')
        ledger.append(f'[{t}] {topic} {verb.format(v=state[topic])} — present: {", ".join(sorted(present))}.')
    updates = []
    while sum(len(l) + 1 for l in lines) < target_chars:
        t += 1
        roll = rng.random()
        if roll < 0.04 and len(present) > 2:
            who = rng.choice(sorted(present)); present.discard(who)
            lines.append(f'[{t}] {who} left the chat.'); ledger.append(f'[{t}] {who} left.')
        elif roll < 0.08 and len(present) < len(people):
            who = rng.choice([p for p in people if p not in present]); present.add(who)
            lines.append(f'[{t}] {who} joined the chat.'); ledger.append(f'[{t}] {who} joined.')
        elif roll < 0.095:
            topic, verb, values = rng.choice(facts)
            new = rng.choice([v for v in values if v != state[topic]])
            state[topic] = new
            for p in people:
                if p in present:
                    knowledge[topic][p] = new
            speaker = rng.choice(sorted(present))
            lines.append(f'[{t}] {speaker}: Change of plan, {topic} {verb.format(v=new)} now.')
            ledger.append(f'[{t}] UPDATE {topic} {verb.format(v=new)} — present: {", ".join(sorted(present))}; '
                          f'absent: {", ".join(sorted(set(people) - present)) or "nobody"}.')
            updates.append(t)
        else:
            lines.append(f'[{t}] {rng.choice(sorted(present))}: {rng.choice(CHATTER)}')
    return {'people': people, 'facts': facts, 'state': state, 'knowledge': knowledge, 'lines': lines,
            'ledger': ledger, 'updates': updates, 'present': sorted(present)}


def pack(world, recent=40):
    header = (f'Social context pack. People: {", ".join(world["people"])}. Domain: personal. '
              'The ledger lists every stated fact and change with who was present to read it.')
    return '\n'.join([header, 'Knowledge ledger:'] + world['ledger'] +
                     ['Recent messages (verbatim):'] + world['lines'][-recent:])


def pack_state(world, recent=40):
    """Pack v2: the host also materialises each person's last-seen value per fact (the
    deterministic bookkeeping Cortex can do from presence logs), so the model reads beliefs
    instead of recomputing them from the whole ledger."""
    rows = []
    for topic, verb, _ in world['facts']:
        current = verb.format(v=world['state'][topic])
        for p in world['people']:
            seen = world['knowledge'][topic][p]
            if seen is None:
                rows.append(f'- {p}: never saw any message about {topic} '
                            f'(does not know that {topic} {current}).')
            elif seen == world['state'][topic]:
                rows.append(f'- {p}: last saw that {topic} {verb.format(v=seen)} '
                            f'(up to date; knows that {topic} {current}).')
            else:
                rows.append(f'- {p}: last saw that {topic} {verb.format(v=seen)}; missed the later change '
                            f'(does not know that {topic} {current}; still believes {topic} {verb.format(v=seen)}).')
    header = (f'Social context pack v2. People: {", ".join(world["people"])}. Domain: personal. '
              'Per-person state is computed from who was present for each message.')
    return '\n'.join([header, 'What each person has seen:'] + rows + ['Knowledge ledger:'] + world['ledger'] +
                     ['Recent messages (verbatim):'] + world['lines'][-recent:])


def items(seed=20261017, worlds=20, target_chars=84000, form='raw'):
    out = []
    forms = {'raw': lambda w: '\n'.join(w['lines']), 'pack': pack, 'pack2': pack_state}
    if form not in forms:
        raise ValueError('form must be raw, pack or pack2')
    for w in range(worlds):
        world = build(seed * 1000 + w, target_chars)
        context = forms[form](world)
        rng = random.Random(seed * 7 + w)
        for topic, verb, _ in world['facts']:
            current = verb.format(v=world['state'][topic])
            knows = [p for p in world['people'] if world['knowledge'][topic][p] == world['state'][topic]]
            unaware = [p for p in world['people'] if p not in knows]
            n = min(2, len(knows), len(unaware))
            if n == 0:
                continue                       # a topic everyone (or nobody) knows cannot be asked fairly
            for p in rng.sample(knows, n) + rng.sample(unaware, n):
                question = f'Does {p} know that {topic} {current}? Answer yes or no.'
                out.append({'id': f'{form}:{w}:{topic}:{p}:know', 'world': w, 'kind': 'knowledge',
                            'messages': [{'role': 'user', 'content': f'{context}\n\n{question}'}],
                            'gold': 'yes' if p in knows else 'no'})
                believed = world['knowledge'][topic][p]
                if believed is not None:
                    out.append({'id': f'{form}:{w}:{topic}:{p}:belief', 'world': w, 'kind': 'belief',
                                'messages': [{'role': 'user', 'content': f'{context}\n\nWhat does {p} believe about {topic}? '
                                                                         'Answer with the value only.'}],
                                'gold': believed, 'other': [v for v in dict.fromkeys(_values(world, topic)) if v != believed]})
    return out


def _values(world, topic):
    return next(values for name, _, values in world['facts'] if name == topic)


def score(items, predictions):
    by_kind = {}
    for item in items:
        pred = predictions[item['id']].lower()
        if item['kind'] == 'knowledge':
            ok = pred.strip().startswith(item['gold'])
        else:
            ok = item['gold'].lower() in pred and not any(o.lower() in pred for o in item['other'])
        by_kind.setdefault(item['kind'], []).append(ok)
    return {k: {'accuracy': sum(v)/len(v), 'n': len(v)} for k, v in by_kind.items()}
