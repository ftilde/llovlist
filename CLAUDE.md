# Tests

Only test behaviour that is not immediately visible from the implementation.

- Worth a test: CRDT merge semantics (concurrency, resurrection, tag/tick-off
  interplay), position resolution and key minting, undo restoring exact state,
  anything where the correct outcome needs thought.
- Not worth a test: a method returning what its one-line body says, a flag being
  set, a stack being cleared, an early return on a missing entry, plumbing.
- Before adding a test ask: could a reader answer this by reading the function
  once? If yes, do not write it.
- When trimming, prefer one test per non-obvious mechanism over several
  variations of the same one.
