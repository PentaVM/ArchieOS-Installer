# IRIS video

Video is represented as bounded ephemeral frames feeding semantic deltas. Queues discard
old frames, revocation clears pending data, and pixel buffers implement zeroing on drop.
Frames are deliberately non-serializable and never enter traces or durable memory. Remote
perception transports semantic deltas rather than raw video.
