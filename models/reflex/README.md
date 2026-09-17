# The local Reflex model, and what taught it

`controls/` is a detector that says where the controls in a window are. It is about five
thousand numbers — 5 KB quantised to int8 — and it looks at one window in about 9 ms on
one core, which is why Archie can ask it on the way to a click.

It never reads anything and it never decides anything. Naming is what an application's own
declarations are for, and what a grounding model is asked for when nothing declares; this
model only says "there is something clickable here". Sight uses it three ways: to put a
grounding model's answer on a real control instead of near one, to find a control again
after a window redrew it, and to say what there is to click at all.

`corpus/` is what taught it: 448 windows and the 3424 controls in them. Nobody labelled
them by hand and no screen of anybody's was involved. Applications that publish their
controls to the accessibility bus label their own captures — `archie-teach` takes the
capture through the stage's snapshot gate, asks the application over that bus where its
controls are, and keeps the pair. The windows came from `display/stage/demo/controls.qml`,
which rearranges itself every second and a half, under Qt's Basic, Fusion and Material
styles.

A tile is 160x96 greys, which is too small to read and large enough to see that something
is a button.

## Doing it again

```bash
# Collect, with the compositor running and the demo window on it:
archie-teach --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock" --key-file … \
    --policy apps.toml --app org.qt-project.qml --corpus models/reflex/corpus --rounds 150

# Train, and score on the windows it was never taught:
cargo run --release -p archie-reflex --bin archie-reflex -- \
    train --corpus models/reflex/corpus --out models/reflex/controls --epochs 200
cargo run --release -p archie-reflex --bin archie-reflex -- \
    check --model models/reflex/controls --corpus models/reflex/corpus
```

The same corpus and the same seed give the same model, on any machine: the shuffle, the
split and the weights all come from `Training::seed`.

## What it scores

On the 89 windows of the corpus it was never taught, at the confidence Sight requires
(800/1000):

| | |
| --- | --- |
| declared controls it found | 740/1000 |
| found near enough to click | 672/1000 |
| things it said that nothing declared | 344/1000 |
| kind right, of what it found | 643/1000 |

The last two are honest upper bounds rather than mistakes: an application declares its
buttons and fields, not its labels, panels and decorations, so a box drawn around
something real that nothing declared counts here as invention. `archie-reflex check`
sweeps the confidence so that trade can be seen rather than assumed.

## What it is not

It is a model trained on one toolkit's windows, on one machine, in an afternoon. It is
enough to show the seam works and to make a click land where a slow model only points; it
is not a general control detector, and nothing in Archie lets it act on its own.
