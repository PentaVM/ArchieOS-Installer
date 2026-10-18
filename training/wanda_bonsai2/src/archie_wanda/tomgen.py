"""Procedural theory-of-mind problems with labels and reasoning traces computed by simulation.

Two owned world types, written with our own templates (never benchmark items):
- object worlds: people enter/leave a place, an object moves between containers, the move is
  witnessed or not, and may be told about afterwards. First- and second-order beliefs about
  the object's location are tracked by simulation.
- conversation worlds: people join and leave a group chat while a fact is updated; who
  heard which version is tracked per message.
Questions: coarse (yes/no) and fine location belief, first and second order; fullness and
accessibility (multi-hop); attitude from a stated preference; knowledge; belief content;
"who knows" lists. Labels are balanced by rejection sampling so that a constant answer
cannot score well. Every target is a short trace (who observed what → beliefs → answer)
for the model's <think> block, then the answer.
"""
from dataclasses import dataclass, field
import hashlib
import random
import re

NAMES = {'en': ['Alice', 'Bruno', 'Carla', 'Diego', 'Elena', 'Farid', 'Greta', 'Hugo', 'Ines', 'Jonah',
                'Kira', 'Leo', 'Maya', 'Nico', 'Olga', 'Pablo', 'Rosa', 'Samir', 'Tara', 'Victor'],
         'es': ['Alba', 'Bernardo', 'Celia', 'Dario', 'Emma', 'Fausto', 'Gala', 'Hector', 'Irene', 'Julian',
                'Karen', 'Lucas', 'Marina', 'Noel', 'Olivia', 'Pedro', 'Raquel', 'Sergio', 'Tania', 'Ulises']}
PLACES = {'en': ['the kitchen', 'the garage', 'the office', 'the studio', 'the cabin', 'the workshop'],
          'es': ['la cocina', 'el garaje', 'la oficina', 'el estudio', 'la cabaña', 'el taller']}
# (noun with article, kind): kind drives accessibility; personal/locked containers are less
# accessible to others, open shelves more.
CONTAINERS = {'en': [('the red box', 'shared'), ('the wooden drawer', 'shared'), ('the blue basket', 'shared'),
                     ('the metal cabinet', 'shared'), ('the open shelf', 'open'), ('the backpack', 'personal'),
                     ('the locked safe', 'locked'), ('the tote bag', 'personal')],
              'es': [('la caja roja', 'shared'), ('el cajón de madera', 'shared'), ('la cesta azul', 'shared'),
                     ('el armario metálico', 'shared'), ('el estante abierto', 'open'), ('la mochila', 'personal'),
                     ('la caja fuerte', 'locked'), ('el bolso', 'personal')]}
OBJECTS = {'en': ['the scarf', 'the notebook', 'the flashlight', 'the camera', 'the harmonica', 'the compass',
                  'the chess set', 'the teapot'],
           'es': ['la bufanda', 'el cuaderno', 'la linterna', 'la cámara', 'la armónica', 'la brújula',
                  'el ajedrez', 'la tetera']}
TOPICS = {'en': [('the trip', 'is in {v}', ['May', 'June', 'July', 'August']),
                 ('the budget', 'is {v}', ['800 euros', '1,200 euros', '950 euros', '2,000 euros']),
                 ('the meeting point', 'is {v}', ['the harbour hall', 'the old library', 'the rooftop', 'the gym'])],
          'es': [('el viaje', 'es en {v}', ['mayo', 'junio', 'julio', 'agosto']),
                 ('el presupuesto', 'es de {v}', ['800 euros', '1.200 euros', '950 euros', '2.000 euros']),
                 ('el punto de encuentro', 'es {v}', ['el salón del puerto', 'la vieja biblioteca', 'la azotea', 'el gimnasio'])]}

T = {
    'en': dict(
        system='You track what each person has seen and heard. Reason briefly, then answer.',
        enter='{p} enters {place}.', leave='{p} leaves {place}.', back='{p} comes back to {place}.',
        start='{o} is in {c}.', move='{p} moves {o} from {a} to {b}.',
        tell='Later, {p} tells {q} that {o} is now in {b}.',
        prefer='{p} {pref} {o}.', pref_like='loves', pref_dislike='dislikes', pref_neutral='does not care about',
        q_coarse='Does {p} think {o} is still in {c}? Answer yes or no.',
        q_coarse2='Does {p} think that {q} believes {o} is still in {c}? Answer yes or no.',
        q_fine='Where will {p} look for {o} first? Answer with the container.',
        q_fine2='Where does {p} think {q} will look for {o}? Answer with the container.',
        q_full='From {p}\'s point of view, does {c} hold more, the same, or less than at the start?',
        q_access='From {p}\'s point of view, is {o} more accessible, equally accessible, or less accessible to {q} than at the start?',
        q_att='Suppose {p} saw it: what is {p}\'s attitude towards {q} moving {o}? Answer positive, neutral, or negative.',
        saw='{p} was present and saw the move.', missed='{p} was not there when {o} was moved.',
        told='{p} missed the move but {q} told {p} about it.', thinks='So {p} thinks {o} is in {c}.',
        so_mover='{p} made the move and knows that {q} {qstate}.', so_made='{p} knows that {q} made the move.',
        so_unaware='{p} does not know about any move, so {p} assumes {q} also thinks {o} is where it was.',
        so_result='So {p} thinks {q} believes {o} is in {c}.',
        qs_saw='saw it', qs_told='was told about it', qs_missed='did not see it',
        full_more='{c} received {o}.', full_less='{o} left {c}.', full_same='{p} saw no change to {c}.',
        acc_personal='{c} is personal, so others cannot freely take things from it.', acc_locked='{c} is locked.',
        acc_open='{c} is open and shared, so anyone can reach it.', acc_same='{p} believes nothing changed for {q}.',
        att='{p} {pref} {o}; putting it in {c} {effect}.', eff_bad='puts it out of {p}\'s reach',
        eff_good='keeps it reachable', eff_none='makes little difference to {p}',
        in_chat='In the group chat: {people}.', join='{p} joins the chat.', left='{p} leaves the chat.',
        say='{p}: {text}', update='update, {topic} {state} now.', small=['Nice weather today.',
        'Who is bringing snacks?', 'Ha, good one.', 'Sounds good to me.', 'I can help with that.'],
        q_know='Does {p} know that {topic} {state}? Answer yes or no.',
        q_who='Who in the chat knows that {topic} {state}? List the names.',
        q_belief='What does {p} believe about {topic}?',
        heard='{p} was in the chat when the update was posted.', not_heard='{p} was not in the chat when the update was posted.',
        heard_old='{p} saw the earlier message that {topic} {old}.',
        believes='{p} believes {topic} {state}.', yes='yes', no='no', answer='Answer: {a}',
        levels=('more', 'the same', 'less'), access=('more accessible', 'equally accessible', 'less accessible'),
        attitude=('positive', 'neutral', 'negative')),
    'es': dict(
        system='Sigues lo que cada persona ha visto y oído. Razona brevemente y responde.',
        enter='{p} entra en {place}.', leave='{p} sale de {place}.', back='{p} vuelve a {place}.',
        start='{o} está en {c}.', move='{p} pasa {o} de {a} a {b}.',
        tell='Más tarde, {p} le cuenta a {q} que {o} está ahora en {b}.',
        prefer='A {p} {pref} {o}.', pref_like='le encanta', pref_dislike='le desagrada', pref_neutral='le da igual',
        q_coarse='¿Cree {p} que {o} sigue en {c}? Responde sí o no.',
        q_coarse2='¿Cree {p} que {q} piensa que {o} sigue en {c}? Responde sí o no.',
        q_fine='¿Dónde buscará primero {p} {o}? Responde con el recipiente.',
        q_fine2='¿Dónde cree {p} que {q} buscará {o}? Responde con el recipiente.',
        q_full='Desde el punto de vista de {p}, ¿{c} contiene más, lo mismo o menos que al principio?',
        q_access='Desde el punto de vista de {p}, ¿{o} es más accesible, igual de accesible o menos accesible para {q} que al principio?',
        q_att='Suponiendo que {p} lo viera, ¿qué actitud tendría hacia que {q} cambie de sitio {o}? Responde positiva, neutral o negativa.',
        saw='{p} estaba presente y vio el cambio.', missed='{p} no estaba cuando se cambió de sitio {o}.',
        told='{p} no vio el cambio, pero {q} se lo contó.', thinks='Así que {p} cree que {o} está en {c}.',
        so_mover='{p} hizo el cambio y sabe que {q} {qstate}.', so_made='{p} sabe que {q} hizo el cambio.',
        so_unaware='{p} no sabe de ningún cambio, así que supone que {q} también cree que {o} sigue donde estaba.',
        so_result='Así que {p} cree que {q} piensa que {o} está en {c}.',
        qs_saw='lo vio', qs_told='se enteró porque se lo contaron', qs_missed='no lo vio',
        full_more='{c} recibió {o}.', full_less='{o} salió de {c}.', full_same='{p} no vio cambios en {c}.',
        acc_personal='{c} es personal, así que los demás no pueden sacar cosas libremente.', acc_locked='{c} está cerrada con llave.',
        acc_open='{c} es abierto y compartido, así que cualquiera llega.', acc_same='{p} cree que nada cambió para {q}.',
        att='A {p} {pref} {o}; llevar {o} a {c} {effect}.', eff_bad='hace que quede fuera del alcance de {p}',
        eff_good='hace que quede a mano', eff_none='le cambia poco a {p}',
        in_chat='En el chat del grupo: {people}.', join='{p} se une al chat.', left='{p} sale del chat.',
        say='{p}: {text}', update='actualización, ahora {topic} {state}.', small=['Qué buen día hace.',
        '¿Quién trae algo de picar?', 'Ja, buenísimo.', 'Me parece bien.', 'Puedo ayudar con eso.'],
        q_know='¿Sabe {p} que {topic} {state}? Responde sí o no.',
        q_who='¿Quién del chat sabe que {topic} {state}? Di los nombres.',
        q_belief='¿Qué cree {p} sobre {topic}?',
        heard='{p} estaba en el chat cuando se publicó la actualización.', not_heard='{p} no estaba en el chat cuando se publicó la actualización.',
        heard_old='{p} vio el mensaje anterior de que {topic} {old}.',
        believes='{p} cree que {topic} {state}.', yes='sí', no='no', answer='Respuesta: {a}',
        levels=('más', 'lo mismo', 'menos'), access=('más accesible', 'igual de accesible', 'menos accesible'),
        attitude=('positiva', 'neutral', 'negativa')),
}


def say(lang, key, **values):
    """Format a template, then fix Spanish contractions and sentence-initial capitals."""
    text = T[lang][key].format(**values)
    if lang == 'es':
        text = re.sub(r'\bde el\b', 'del', text)
        text = re.sub(r'\ba el\b', 'al', text)
    return text[:1].upper() + text[1:]


@dataclass
class Problem:
    world_id: str
    kind: str
    lang: str
    story: str
    question: str
    trace: list
    answer: str
    label: str            # normalised label used for balancing
    meta: dict = field(default_factory=dict)
    mode: str = 'think'   # 'think': trace in <think> then the answer; 'direct': empty think, answer only

    def messages(self):
        return [{'role': 'system', 'content': T[self.lang]['system']},
                {'role': 'user', 'content': self.story + '\n\n' + self.question}]

    def target(self):
        """Completion after the assistant's opened think block (think) or after an empty one (direct).

        The answer is the bare label, as benchmark parsers expect; no "Answer:" prefix.
        """
        if self.mode == 'direct':
            return self.answer
        return ' '.join(self.trace) + '\n</think>\n\n' + self.answer


class ObjectWorld:
    def __init__(self, rng, lang, world_id, force=None):
        self.lang, self.id = lang, world_id
        self.mover, self.watcher, self.third = rng.sample(NAMES[lang], 3)
        self.place, self.obj = rng.choice(PLACES[lang]), rng.choice(OBJECTS[lang])
        self.start, self.dest = rng.sample(CONTAINERS[lang], 2)
        self.preference = rng.choice(['like', 'dislike', 'neutral'])
        s = lambda key, **v: say(lang, key, **v)
        lines = [s('enter', p=self.mover, place=self.place), s('enter', p=self.watcher, place=self.place),
                 s('start', o=self.obj, c=self.start[0]),
                 s('prefer', p=self.watcher, o=self.obj, pref=T[lang]['pref_' + self.preference])]
        present = {self.mover, self.watcher}
        # Every random decision is drawn in a fixed order, then `force` may override it, so a
        # forced twin differs from its original in exactly the forced variable(s).
        draws = {'watcher_leaves': rng.random() < 0.5, 'third': rng.random() < 0.5,
                 'tell': rng.random() < 0.35, 'back': rng.random() < 0.5}
        draws.update(force or {})
        self.draws = draws
        if draws['watcher_leaves']:
            lines.append(s('leave', p=self.watcher, place=self.place)); present.discard(self.watcher)
        self.third_in_story = draws['third']
        if self.third_in_story:
            lines.append(s('enter', p=self.third, place=self.place)); present.add(self.third)
        lines.append(s('move', p=self.mover, o=self.obj, a=self.start[0], b=self.dest[0]))
        self.saw = {p: p in present for p in (self.mover, self.watcher, self.third)}
        self.told = set()
        if not self.saw[self.watcher] and draws['tell']:
            lines.append(s('tell', p=self.mover, q=self.watcher, o=self.obj, b=self.dest[0]))
            self.told.add(self.watcher)
        if draws['watcher_leaves'] and draws['back']:
            lines.append(s('back', p=self.watcher, place=self.place))
        self.lines = lines

    def knows_move(self, p):
        return self.saw[p] or p in self.told

    def thinks_other_knows(self, p, q):
        """p's model of q's belief, from what p observed (only mover/watcher pairs are asked)."""
        if not self.knows_move(p):
            return False
        if q == self.mover:
            return True                            # p knows the move and q is the one who made it
        if p == self.mover:
            return self.knows_move(q)              # the mover saw who was present and whom they told
        return self.saw[q]

    def story(self):
        return ' '.join(self.lines)

    def problems(self):
        lang, o, out = self.lang, self.obj, []
        s = lambda key, **v: say(lang, key, **v)
        t = T[lang]
        start, dest = self.start[0], self.dest[0]
        box = lambda moved: dest if moved else start

        def why(p):
            if self.saw[p]:
                return s('saw', p=p)
            if p in self.told:
                return s('told', p=p, q=self.mover)
            return s('missed', p=p, o=o)

        def add(kind, question, trace, answer, label):
            out.append(Problem(self.id, kind, lang, self.story(), question, trace, answer, label))

        people = [self.watcher, self.mover] + ([self.third] if self.third_in_story else [])
        for p in people:
            moved = self.knows_move(p)
            trace = [why(p), s('thinks', p=p, o=o, c=box(moved))]
            add('location_coarse_fo', s('q_coarse', p=p, o=o, c=start), trace,
                t['no'] if moved else t['yes'], 'no' if moved else 'yes')
            add('location_fine_fo', s('q_fine', p=p, o=o), trace, box(moved), 'moved' if moved else 'start')
        for p, q in ((self.mover, self.watcher), (self.watcher, self.mover)):
            moved = self.thinks_other_knows(p, q)
            if not self.knows_move(p):
                first = s('so_unaware', p=p, q=q, o=o)
            elif p == self.mover:
                state = t['qs_saw'] if self.saw[q] else (t['qs_told'] if q in self.told else t['qs_missed'])
                first = s('so_mover', p=p, q=q, qstate=state)
            else:
                first = s('so_made', p=p, q=q)
            trace = [why(p), first, s('so_result', p=p, q=q, o=o, c=box(moved))]
            add('location_coarse_so', s('q_coarse2', p=p, q=q, o=o, c=start), trace,
                t['no'] if moved else t['yes'], 'no' if moved else 'yes')
            add('location_fine_so', s('q_fine2', p=p, q=q, o=o), trace, box(moved), 'moved' if moved else 'start')
        for p in (self.watcher, self.mover):
            moved = self.knows_move(p)
            for container, gains in ((start, False), (dest, True)):
                if not moved:
                    label, why_full = 1, s('full_same', p=p, c=container)
                elif gains:
                    label, why_full = 0, s('full_more', c=container, o=o)
                else:
                    label, why_full = 2, s('full_less', c=container, o=o)
                add('fullness', s('q_full', p=p, c=container), [why(p), why_full], t['levels'][label],
                    ('more', 'same', 'less')[label])
            other = self.watcher if p == self.mover else self.mover
            closed_before = self.start[1] in ('personal', 'locked')
            closed_after = self.dest[1] in ('personal', 'locked')
            if not moved or closed_before == closed_after:
                label, why_access = 1, s('acc_same', p=p, q=other)
            elif closed_after:
                label, why_access = 2, s('acc_' + self.dest[1], c=dest)
            else:                                  # out of a closed container into a shared one
                label, why_access = 0, s('acc_open', c=dest)
            add('accessibility', s('q_access', p=p, o=o, q=other), [why(p), why_access], t['access'][label],
                ('more', 'same', 'less')[label])
        out_of_reach = self.dest[1] in ('personal', 'locked') and self.start[1] not in ('personal', 'locked')
        into_reach = self.dest[1] in ('shared', 'open') and self.start[1] in ('personal', 'locked')
        if self.preference == 'like':
            label, effect = (2, 'eff_bad') if out_of_reach else ((0, 'eff_good') if into_reach else (1, 'eff_none'))
        elif self.preference == 'dislike':
            label, effect = (0, 'eff_bad') if out_of_reach else ((2, 'eff_good') if into_reach else (1, 'eff_none'))
        else:
            label, effect = 1, 'eff_none'
        add('attitude', s('q_att', p=self.watcher, q=self.mover, o=o),
            [s('att', p=self.watcher, o=o, c=dest, pref=t['pref_' + self.preference],
               effect=t[effect].format(p=self.watcher))],
            t['attitude'][label], ('positive', 'neutral', 'negative')[label])
        return out


class ConversationWorld:
    def __init__(self, rng, lang, world_id):
        self.lang, self.id = lang, world_id
        s = lambda key, **v: say(lang, key, **v)
        people = rng.sample(NAMES[lang], 4)
        self.topic, self.state, values = rng.choice(TOPICS[lang])
        self.old, self.new = rng.sample(values, 2)
        present = set(people[:3])
        opening = f'{self.topic} {self.state.format(v=self.old)}.'
        lines = [s('in_chat', people=', '.join(people[:3])),
                 s('say', p=people[1], text=opening[:1].upper() + opening[1:])]
        self.heard_old = set(present)
        if rng.random() < 0.7:
            leaver = rng.choice(people[1:3]); present.discard(leaver); lines.append(s('left', p=leaver))
        if rng.random() < 0.6:
            present.add(people[3]); lines.append(s('join', p=people[3]))
        lines.append(s('say', p=rng.choice(sorted(present)), text=rng.choice(T[lang]['small'])))
        lines.append(s('say', p=people[0], text=s('update', topic=self.topic, state=self.state.format(v=self.new))))
        self.heard = set(present)
        absent = [p for p in people if p not in present]
        if absent and rng.random() < 0.5:
            joiner = rng.choice(absent); lines.append(s('join', p=joiner))
            lines.append(s('say', p=joiner, text=rng.choice(T[lang]['small'])))
        self.lines = lines
        self.mentioned = [p for p in people if any(p in line for line in lines)]

    def story(self):
        return '\n'.join(self.lines)

    def problems(self):
        lang, out = self.lang, []
        s = lambda key, **v: say(lang, key, **v)
        new, old = self.state.format(v=self.new), self.state.format(v=self.old)
        t = T[lang]
        for p in self.mentioned:
            knows = p in self.heard
            heard = s('heard' if knows else 'not_heard', p=p)
            out.append(Problem(self.id, 'knowledge', lang, self.story(), s('q_know', p=p, topic=self.topic, state=new),
                               [heard], t['yes'] if knows else t['no'], 'yes' if knows else 'no'))
            if knows or p in self.heard_old:
                trace = [heard] + ([] if knows else [s('heard_old', p=p, topic=self.topic, old=old)])
                belief = s('believes', p=p, topic=self.topic, state=new if knows else old)
                out.append(Problem(self.id, 'belief', lang, self.story(), s('q_belief', p=p, topic=self.topic),
                                   trace + [belief], belief, 'new' if knows else 'old'))
        names = [p for p in self.mentioned if p in self.heard]
        out.append(Problem(self.id, 'who_knows', lang, self.story(), s('q_who', topic=self.topic, state=new),
                           [s('heard', p=p) for p in names], ', '.join(names), str(len(names))))
        return out


def minimal_pairs(seed, lang, world_id):
    """Twin object worlds differing only in whether the watcher left before the move (and no
    telling), so the watcher's beliefs flip while everything else is identical. Returns
    (sensitive, invariant): question pairs whose answer flips, and pairs that must not."""
    first = ObjectWorld(random.Random(seed), lang, world_id, force={'tell': False})
    twin = ObjectWorld(random.Random(seed), lang, world_id,
                       force={'tell': False, 'watcher_leaves': not first.draws['watcher_leaves']})
    left = {(p.kind, p.question): p for p in first.problems()}
    right = {(p.kind, p.question): p for p in twin.problems()}
    sensitive, invariant = [], []
    for key in sorted(left.keys() & right.keys()):              # sorted: set order varies per process
        a, b = left[key], right[key]
        tag = hashlib.sha256(key[1].encode()).hexdigest()[:8]
        a.meta = dict(a.meta, pair=f'{world_id}:{key[0]}:{tag}', side='a')
        b.meta = dict(b.meta, pair=a.meta['pair'], side='b')
        (sensitive if a.label != b.label else invariant).append((a, b))
    return sensitive, invariant


def generate_v2(count, seed=20261018, langs=('en', 'es'), pair_share=0.5, direct_share=0.5):
    """g2 mix: half minimal counterfactual pairs (mostly answer-flipping, some invariant),
    half balanced single problems; each problem in think or direct mode."""
    rng = random.Random(seed)
    out, world = [], 0
    pair_quota = int(count * pair_share)
    while len(out) < pair_quota and world < count * 50:
        lang = langs[world % len(langs)]
        sensitive, invariant = minimal_pairs(rng.randrange(2**31), lang, f'p{seed}-{world}')
        world += 1
        rng.shuffle(sensitive); rng.shuffle(invariant)
        for a, b in sensitive[:2] + invariant[:1]:
            if len(out) + 2 > pair_quota:
                break
            out += [a, b]
    out += generate(count - len(out), seed=seed + 1, langs=langs)
    for problem in out:
        problem.mode = 'direct' if rng.random() < direct_share else 'think'
    return out


LABELS = {'location_coarse_fo': ('yes', 'no'), 'location_coarse_so': ('yes', 'no'), 'knowledge': ('yes', 'no'),
          'location_fine_fo': ('moved', 'start'), 'location_fine_so': ('moved', 'start'),
          'fullness': ('more', 'same', 'less'), 'accessibility': ('more', 'same', 'less'),
          'attitude': ('positive', 'neutral', 'negative'), 'belief': ('new', 'old'),
          'faux_pas_aware': ('yes', 'no'), 'commitment': ('firm', 'tentative', 'declining'),
          'emotional_effect': ('pressured', 'supported', 'unbothered'),
          'pack_knowledge': ('yes', 'no'), 'pack_belief': ('new', 'old')}

CONSTRAINTS = {
    'en': [
        ('exhausted from a double shift', 'an late-night rehearsal', 'is exhausted after working a double shift'),
        ('worried about tight finances this month', 'an expensive weekend retreat', 'is worried about tight finances this month'),
        ('recovering from a migraine', 'a loud concert downtown', 'is recovering from a migraine'),
        ('on deadline for a client delivery', 'an all-afternoon hike', 'is on a strict deadline for a client delivery'),
    ],
    'es': [
        ('agotado tras un turno doble', 'un ensayo nocturno hasta tarde', 'está agotado tras trabajar un turno doble'),
        ('preocupado por los gastos de este mes', 'una escapada cara de fin de semana', 'está preocupado por los gastos de este mes'),
        ('recuperándose de una migraña', 'un concierto ruidoso en el centro', 'se está recuperando de una migraña'),
        ('con una entrega urgente del trabajo', 'una caminata de toda la tarde', 'tiene una entrega urgente del trabajo'),
    ],
}

REPLIES = {
    'en': {
        'firm': ('Count me in, I cleared my schedule and I will definitely be there.', 'firm'),
        'tentative': ('Maybe, let me see how things go and check back later.', 'tentative'),
        'declining': ('You all go ahead without me this time, another week would be much easier for me.', 'declining'),
    },
    'es': {
        'firm': ('Cuenten conmigo, ya liberé mi agenda y seguro que voy.', 'firme'),
        'tentative': ('Quizás, déjame ver cómo avanzo y les aviso más tarde.', 'tentativo'),
        'declining': ('Vayan ustedes sin mí esta vez, otra semana me vendría mucho mejor.', 'rechazando'),
    },
}


class BehavioralWorld:
    """Social and behavioral psychology simulation: private constraint disclosure, witness
    asymmetry (who heard the constraint), polite indirectness / hedging vs firm commitment,
    faux-pas awareness, and emotional impact of pushing vs accommodating."""

    def __init__(self, rng, lang, world_id, force=None):
        self.lang, self.id = lang, world_id
        self.speaker, self.proposer, self.observer = rng.sample(NAMES[lang], 3)
        self.short_constraint, self.proposal, self.clause = rng.choice(CONSTRAINTS[lang])
        draws = {
            'proposer_present_at_disclosure': rng.random() < 0.5,
            'observer_present_at_disclosure': rng.random() < 0.5,
            'reply_style': rng.choice(['firm', 'tentative', 'declining']),
            'proposer_response': rng.choice(['push', 'support', 'neutral']),
        }
        draws.update(force or {})
        self.draws = draws
        if lang == 'en':
            listeners = [self.observer] + ([self.proposer] if draws['proposer_present_at_disclosure'] else [])
            absent = [] if draws['proposer_present_at_disclosure'] else [f'{self.proposer} is not in the room yet.']
            lines = [
                f'Earlier, while talking with {", ".join(listeners)}, {self.speaker} mentions that {self.speaker} {self.clause}.',
                *absent,
                f'Later, {self.proposer} joins and proposes {self.proposal} for tonight.',
                f'{self.speaker} replies: "{REPLIES[lang][draws["reply_style"]][0]}"',
            ]
            if draws['proposer_response'] == 'push':
                lines.append(f'{self.proposer} insists that everyone must join tonight with no excuses.')
            elif draws['proposer_response'] == 'support':
                lines.append(f'{self.proposer} says there is zero pressure and offers a low-key option next week.')
            else:
                lines.append(f'{self.proposer} notes the time and leaves the plan open.')
        else:
            listeners = [self.observer] + ([self.proposer] if draws['proposer_present_at_disclosure'] else [])
            absent = [] if draws['proposer_present_at_disclosure'] else [f'{self.proposer} todavía no está en la sala.']
            lines = [
                f'Antes, hablando con {", ".join(listeners)}, {self.speaker} comenta que {self.clause}.',
                *absent,
                f'Más tarde, {self.proposer} se une y propone {self.proposal} para esta noche.',
                f'{self.speaker} responde: "{REPLIES[lang][draws["reply_style"]][0]}"',
            ]
            if draws['proposer_response'] == 'push':
                lines.append(f'{self.proposer} insiste en que todos tienen que ir esta noche sin excusas.')
            elif draws['proposer_response'] == 'support':
                lines.append(f'{self.proposer} dice que no hay ninguna presión y ofrece una opción tranquila la próxima semana.')
            else:
                lines.append(f'{self.proposer} anota la hora y deja el plan abierto.')
        self.lines = lines

    def story(self):
        return ' '.join(self.lines)

    def problems(self):
        lang, out = self.lang, []
        t = T[lang]
        aware = self.draws['proposer_present_at_disclosure']
        style = self.draws['reply_style']
        resp = self.draws['proposer_response']
        if lang == 'en':
            q_fp = (f'Did {self.proposer} know that {self.speaker} {self.clause} when proposing '
                    f'{self.proposal}? Answer yes or no.')
            tr_fp = [
                f'{self.proposer} {"was present" if aware else "was not in the room"} when {self.speaker} mentioned being {self.short_constraint}.',
                f'So {self.proposer} {"knew" if aware else "did not know"} about {self.speaker}\'s constraint.',
            ]
            q_comm = (f'Based on {self.speaker}\'s reply, is {self.speaker}\'s commitment to {self.proposal} '
                      'firm, tentative, or declining?')
            tr_comm = [
                f'{self.speaker} replied: "{REPLIES[lang][style][0]}"',
                f'That wording is a {style} signal regarding attendance.',
            ]
            if style == 'firm':
                eff_ans, eff_lbl = 'unbothered', 'unbothered'
                tr_eff = [f'{self.speaker} explicitly committed and cleared their schedule, so {self.speaker} feels unbothered.']
            elif resp == 'push':
                eff_ans, eff_lbl = 'pressured', 'pressured'
                tr_eff = [f'{self.speaker} signalled "{style}" while {self.short_constraint}, and {self.proposer} insisted with no excuses, leaving {self.speaker} feeling pressured.']
            elif resp == 'support':
                eff_ans, eff_lbl = 'supported', 'supported'
                tr_eff = [f'{self.proposer} removed pressure and offered an alternative, so {self.speaker} feels supported.']
            else:
                eff_ans, eff_lbl = 'unbothered', 'unbothered'
                tr_eff = [f'{self.proposer} left the plan open without pressuring {self.speaker}, so {self.speaker} feels unbothered.']
            q_eff = (f'After {self.proposer}\'s follow-up, is {self.speaker} most likely to feel '
                     'pressured, supported, or unbothered?')
        else:
            q_fp = (f'¿Sabía {self.proposer} que {self.speaker} {self.clause} al proponer '
                    f'{self.proposal}? Responde sí o no.')
            tr_fp = [
                f'{self.proposer} {"estaba presente" if aware else "no estaba en la sala"} cuando {self.speaker} comentó que estaba {self.short_constraint}.',
                f'Así que {self.proposer} {"sí sabía" if aware else "no sabía"} de la situación de {self.speaker}.',
            ]
            q_comm = (f'Según la respuesta de {self.speaker}, ¿su compromiso con {self.proposal} es '
                      'firme, tentativo o rechazando?')
            tr_comm = [
                f'{self.speaker} respondió: "{REPLIES[lang][style][0]}"',
                f'Esa respuesta expresa un compromiso {REPLIES[lang][style][1]}.',
            ]
            if style == 'firm':
                eff_ans, eff_lbl = 'tranquilo', 'unbothered'
                tr_eff = [f'{self.speaker} confirmó con seguridad que liberó su agenda, así que se siente tranquilo.']
            elif resp == 'push':
                eff_ans, eff_lbl = 'presionado', 'pressured'
                tr_eff = [f'{self.speaker} dio una señal no firme estando {self.short_constraint} y {self.proposer} insistió sin excusas, por lo que {self.speaker} se siente presionado.']
            elif resp == 'support':
                eff_ans, eff_lbl = 'apoyado', 'supported'
                tr_eff = [f'{self.proposer} quitó toda presión y ofreció alternativa, así que {self.speaker} se siente apoyado.']
            else:
                eff_ans, eff_lbl = 'tranquilo', 'unbothered'
                tr_eff = [f'{self.proposer} dejó el plan abierto sin insistir, así que {self.speaker} se siente tranquilo.']
            q_eff = (f'Tras la reacción de {self.proposer}, ¿es más probable que {self.speaker} se sienta '
                     'presionado, apoyado o tranquilo?')

        out.append(Problem(self.id, 'faux_pas_aware', lang, self.story(), q_fp, tr_fp,
                           t['yes'] if aware else t['no'], 'yes' if aware else 'no'))
        out.append(Problem(self.id, 'commitment', lang, self.story(), q_comm, tr_comm,
                           REPLIES[lang][style][1], style))
        out.append(Problem(self.id, 'emotional_effect', lang, self.story(), q_eff, tr_eff,
                           eff_ans, eff_lbl))
        return out


class PackStateWorld:
    """Compacted Social Context Pack v2 world (train-length counterpart to longchat.pack_state)
    so the model learns to read materialised per-person epistemic state and ledger updates."""

    def __init__(self, rng, lang, world_id, force=None):
        self.lang, self.id = lang, world_id
        self.people = rng.sample(NAMES[lang], 4)
        self.topic, self.state, values = rng.choice(TOPICS[lang])
        self.old, self.new = rng.sample(values, 2)
        draws = {'target_present': rng.random() < 0.5}
        draws.update(force or {})
        self.draws = draws
        self.target_person = self.people[1]
        present_initial = sorted(self.people[:3])
        present_update = sorted([self.people[0], self.people[2]] + ([self.target_person] if draws['target_present'] else [self.people[3]]))
        absent_update = sorted(set(self.people) - set(present_update))
        old_clause = self.state.format(v=self.old)
        new_clause = self.state.format(v=self.new)
        self.seen = {}
        for p in self.people:
            if p in present_update:
                self.seen[p] = self.new
            elif p in present_initial:
                self.seen[p] = self.old
            else:
                self.seen[p] = None
        rows = []
        for p in self.people:
            seen_val = self.seen[p]
            if seen_val is None:
                rows.append(f'- {p}: never saw any message about {self.topic} (does not know that {self.topic} {new_clause}).')
            elif seen_val == self.new:
                rows.append(f'- {p}: last saw that {self.topic} {self.state.format(v=seen_val)} (up to date; knows that {self.topic} {new_clause}).')
            else:
                rows.append(f'- {p}: last saw that {self.topic} {self.state.format(v=seen_val)}; missed the later change '
                            f'(does not know that {self.topic} {new_clause}; still believes {self.topic} {self.state.format(v=seen_val)}).')
        ledger = [
            f'[1] {self.topic} {old_clause} — present: {", ".join(present_initial)}.',
            f'[4] UPDATE {self.topic} {new_clause} — present: {", ".join(present_update)}; absent: {", ".join(absent_update) or "nobody"}.',
        ]
        recent = [
            f'[3] {present_update[0]}: {rng.choice(T["en"]["small"])}',
            f'[4] {self.people[0]}: Change of plan, {self.topic} {new_clause} now.',
        ]
        header = (f'Social context pack v2. People: {", ".join(self.people)}. Domain: personal. '
                  'Per-person state is computed from who was present for each message.')
        self.text = '\n'.join([header, 'What each person has seen:'] + rows +
                              ['Knowledge ledger:'] + ledger + ['Recent messages (verbatim):'] + recent)

    def story(self):
        return self.text

    def problems(self):
        out = []
        new_clause = self.state.format(v=self.new)
        p = self.target_person
        knows = self.seen[p] == self.new
        seen_val = self.seen[p]
        q_know = f'Does {p} know that {self.topic} {new_clause}? Answer yes or no.'
        tr_know = [
            f'In the per-person state, {p} {"last saw the current update and is up to date" if knows else f"missed the later change and still believes {self.topic} {self.state.format(v=seen_val)}"}.',
            f'So {p} {"knows" if knows else "does not know"} that {self.topic} {new_clause}.',
        ]
        out.append(Problem(self.id, 'pack_knowledge', 'en', self.story(), q_know, tr_know,
                           'yes' if knows else 'no', 'yes' if knows else 'no'))
        q_bel = f'What does {p} believe about {self.topic}? Answer with the value only.'
        tr_bel = [
            f'According to what {p} has seen, {p} last saw that {self.topic} {self.state.format(v=seen_val)}.',
            f'Therefore {p} believes the value is {seen_val}.',
        ]
        out.append(Problem(self.id, 'pack_belief', 'en', self.story(), q_bel, tr_bel,
                           seen_val, 'new' if knows else 'old'))
        return out


def behavioral_minimal_pairs(seed, lang, world_id):
    """Twin behavioral worlds differing in disclosure presence and reply style/response so
    faux-pas awareness, commitment reliability, and emotional impact flip cleanly."""
    rng = random.Random(seed)
    first = BehavioralWorld(random.Random(seed), lang, world_id,
                            force={'reply_style': 'tentative', 'proposer_response': 'push'})
    twin = BehavioralWorld(random.Random(seed), lang, world_id,
                           force={'proposer_present_at_disclosure': not first.draws['proposer_present_at_disclosure'],
                                  'reply_style': 'firm' if rng.random() < 0.5 else 'declining',
                                  'proposer_response': 'support'})
    left = {(p.kind, p.question): p for p in first.problems()}
    right = {(p.kind, p.question): p for p in twin.problems()}
    sensitive, invariant = [], []
    for key in sorted(left.keys() & right.keys()):
        a, b = left[key], right[key]
        tag = hashlib.sha256(key[1].encode()).hexdigest()[:8]
        a.meta = dict(a.meta, pair=f'{world_id}:{key[0]}:{tag}', side='a')
        b.meta = dict(b.meta, pair=a.meta['pair'], side='b')
        (sensitive if a.label != b.label else invariant).append((a, b))
    return sensitive, invariant


def pack_minimal_pairs(seed, world_id):
    """Twin PackStateWorld problems differing only in whether the target person was present
    for the update."""
    first = PackStateWorld(random.Random(seed), 'en', world_id, force={'target_present': True})
    twin = PackStateWorld(random.Random(seed), 'en', world_id, force={'target_present': False})
    left = {(p.kind, p.question): p for p in first.problems()}
    right = {(p.kind, p.question): p for p in twin.problems()}
    sensitive = []
    for key in sorted(left.keys() & right.keys()):
        a, b = left[key], right[key]
        tag = hashlib.sha256(key[1].encode()).hexdigest()[:8]
        a.meta = dict(a.meta, pair=f'{world_id}:{key[0]}:{tag}', side='a')
        b.meta = dict(b.meta, pair=a.meta['pair'], side='b')
        if a.label != b.label:
            sensitive.append((a, b))
    return sensitive


def generate_v3(count, seed=20261019, langs=('en', 'es'), pair_share=0.55, direct_share=0.45):
    """g3 mix: counterfactual minimal pairs across ObjectWorld, BehavioralWorld, and
    PackStateWorld plus balanced single problems across all four world families."""
    rng = random.Random(seed)
    out, world = [], 0
    pair_quota = int(count * pair_share)
    while len(out) < pair_quota and world < count * 50:
        lang = langs[world % len(langs)]
        subseed = rng.randrange(2**31)
        if world % 4 == 0:
            sensitive, invariant = behavioral_minimal_pairs(subseed, lang, f'bp{seed}-{world}')
        elif world % 4 == 1:
            sensitive, invariant = pack_minimal_pairs(subseed, f'pp{seed}-{world}'), []
        else:
            sensitive, invariant = minimal_pairs(subseed, lang, f'p{seed}-{world}')
        world += 1
        rng.shuffle(sensitive); rng.shuffle(invariant)
        for a, b in sensitive[:2] + invariant[:1]:
            if len(out) + 2 > pair_quota:
                break
            out += [a, b]
    out += generate(count - len(out), seed=seed + 1, langs=langs, include_behavioral=True)
    for problem in out:
        problem.mode = 'direct' if rng.random() < direct_share else 'think'
    return out


def generate(count, seed=20261017, langs=('en', 'es'), per_world=4, slack=2, include_behavioral=False):
    """`count` problems from fresh worlds; within each (kind, language) every label stays
    within `slack` of the rarest one, so a constant answer cannot exceed chance by much."""
    rng = random.Random(seed)
    counts, out, world = {}, [], 0
    while len(out) < count and world < count * 200:
        lang = langs[world % len(langs)]
        if include_behavioral:
            mod = world % 5
            cls = ConversationWorld if mod == 0 else (BehavioralWorld if mod == 1 else (PackStateWorld if mod == 2 else ObjectWorld))
        else:
            cls = ConversationWorld if world % 3 == 0 else ObjectWorld
        problems = cls(random.Random(rng.random()), lang, f'w{seed}-{world}').problems()
        world += 1
        rng.shuffle(problems)
        taken = 0
        for problem in problems:
            if taken >= per_world or len(out) >= count:
                break
            if problem.kind in LABELS:
                labels = counts.setdefault((problem.kind, problem.lang), dict.fromkeys(LABELS[problem.kind], 0))
                if labels[problem.label] > min(labels.values()) + slack - 1:
                    continue
                labels[problem.label] += 1
            elif rng.random() < 0.5:               # who_knows lists: unbalanced count, keep fewer
                continue
            out.append(problem); taken += 1
    return out
