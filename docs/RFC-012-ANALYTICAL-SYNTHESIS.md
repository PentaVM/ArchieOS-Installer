# RFC-012 — Bounded analytical synthesis

Status: implemented kernel contract; runtime model adapters pending

An analytical model is useful when it separates a claim from the evidence for that claim.
It becomes unreliable when repeated frames, reposted articles or its own confidence score
are treated as new evidence. Cerebra therefore places deterministic synthesis around every
future neural head.

```text
Sentry / Wanda / providers
          │
          v
sanitized signals with provenance and expiry
          │
          ├── consent and provider-policy filter
          ├── age, reliability and relevance filter
          ├── one vote per independent source root
          ├── source self-contradiction quarantine
          └── explicit support / contradiction ledger
                            │
                            v
   unavailable | insufficient | supported | refuted | disputed
                            │
                            v
          neural interpretation (advisory only)
```

`archie-memory::analysis` implements this boundary with integer permille arithmetic and
bounded collections. The same validated inputs produce the same assessment on every node.

## Evidence rules

1. A signal contains opaque evidence references, digests, a semantic key and a primary
   provenance root. It has no field for audio, pixels, transcript text or narrative prose.
2. Analysis permission, consent, expiry, maximum age, minimum reliability and minimum
   relevance are checked before synthesis.
3. One primary source receives at most one supporting or contradicting vote. A hundred
   camera frames or reposts from the same root therefore remain one source.
4. If one source both supports and contradicts the hypothesis, both votes are quarantined.
   If independent roots disagree, the result is `disputed`; averaging may not erase the
   disagreement.
5. `supported` and `refuted` require at least two independent roots by default. Missing
   evidence is `unavailable`, and one-source evidence is `insufficient_evidence`.
6. Recency changes evidence strength but not the number of independent sources. Strength
   is an auditable ranking measure, not a probability or factual confidence claim.

## Neural proposal gate

A future relevance, contradiction or outcome head may return a `ModelProposal`. The gate
requires the proposal to cite enough independent evidence that survived synthesis, clear
the configured model-score floor and agree with the assessment direction. Disputed,
insufficient, uncertain or cherry-picked evidence forces abstention. Fabricated
references or a direction contrary to admitted evidence are rejected.

An accepted proposal remains `advisory`; it does not write a graph fact, change policy,
identify a person, grant an action or become training supervision. Promotion requires the
existing claim-verification and graph-writer paths.

## Current implementation limit

The synthesis kernel, wire schema and deterministic tests are implemented. No learned
checkpoint, model adapter or automatic graph publication is claimed. Runtime integration
must convert provider records into `analytical-signal.schema.json` after the learning and
privacy admission boundary, then persist the assessment and its per-signal decisions in
PostgreSQL.
