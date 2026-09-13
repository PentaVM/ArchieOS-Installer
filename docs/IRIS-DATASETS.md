# IRIS datasets

The operational probe pins and streams three Hugging Face repositories without a full
download: Voxel51 FiftyOne GUI Grounding, ScreenSpot and AllenAI MolmoPoint-GUISyn.
ScreenSpot is evaluation-oriented and is not mixed into training. The first trained
checkpoint uses only `archie-synthetic-v1`.

DocLayNet remains a recommended document corpus but was removed from the live probe after
the Dataset Server failed. ScreenSpot-Pro is evaluation-only and excluded from training.
Open Images requires per-image license filtering before any use.
