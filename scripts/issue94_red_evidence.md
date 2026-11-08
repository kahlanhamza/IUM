RED evidence — #94 (the speed cap), at HEAD `6a96623`

The pins (`ium-core/tests/rate_cap.rs` — the bucket's virtual-time pins +
the manager-wiring module) cannot compile at HEAD:

```
error[E0432]: unresolved import `ium_core::rate`
error[E0407]: method `create_session` is not a member of trait `ium_core::TransferAdapter`
error[E0425]: cannot find type `ChunkPlan` in module `ium_core::chunk`
error[E0425]: cannot find type `ChunkConfirm` in module `ium_core::chunk`
```

`ium_core::rate` does not exist (grep: the only `rate limit` mentions in
core are comments), and the manager has no `set_rate_cap`. Nothing in the
engine can be polite about bandwidth today. Implementation next per the
design note (comment 6726274873): the bucket, the pre-upload acquire inside
a cancellation-safe select! (eviction-safe by construction), the
policy-knob persistence story, the shell card.
