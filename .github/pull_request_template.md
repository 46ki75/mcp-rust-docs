## Summary

<!-- 1-3 bullets: what changed and why. Reference the issue if there is one. -->

## Test plan

- [ ] `just ci` passes locally
- [ ] New behavior is covered at the right test tier

## Notes for reviewers

<!--
Anything non-obvious. A few prompts:
- Which layer was touched (repository / use case / tool)? Type isolation across layer
  boundaries is a deliberate invariant — flag any cross-layer type sharing.
- New workspace dep? It should live in the root `[workspace.dependencies]`.
- Upstream contract or schema change? Call out whether `tests/live.rs` needs an update.
- Anything that affects stdio transport: no `tracing` output may go to stdout.

Delete this block if nothing applies.
-->
