# IRIS Scene Graph

Each source owns typed nodes, stable track IDs, parent/child relations, integer geometry and
a monotonically increasing revision. Mutations are atomic and reject stale writers,
dangling relations and cycles. Fixed-point affine transforms convert crop, window and
screen coordinates reproducibly without platform-dependent floating point.

Damage regions are connected components expanded to semantic parents such as an input or
modal. Caret noise and unchanged frames produce no work.
