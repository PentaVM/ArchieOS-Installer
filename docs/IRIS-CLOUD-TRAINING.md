# IRIS cloud training

Foundry emits a real Nebius job plan with explicit image, command, project, storage,
checkpoint URI, GPU-hour and cost ceilings. Dry-run is the default. Execution requires the
Nebius CLI, credentials and explicit project/storage/budget configuration; none are stored
in either repository.

No cloud job was launched. Cloud preemption/resume and idle shutdown are therefore
**BLOCKED**; local checkpoint/resume is implemented and tested.
