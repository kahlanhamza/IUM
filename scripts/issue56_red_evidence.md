# #56 RED evidence — the Feishu Drive adapter contract fails to compile at HEAD `320322e`

```
error[E0432]: unresolved import `ium_core::adapter_feishu`
  --> ium-core/tests/adapter_feishu_contract.rs:14:15
error[E0599]: no associated function or constant named `plan_uniform_tail` found for struct `ChunkPool` in the current scope
error[E0599]: no associated function or constant named `plan_uniform_tail` found for struct `ChunkPool` in the current scope
error[E0599]: no associated function or constant named `plan_uniform_tail` found for struct `ChunkPool` in the current scope
error[E0599]: no associated function or constant named `plan_uniform_tail` found for struct `ChunkPool` in the current scope
error[E0599]: no associated function or constant named `new_uniform` found for struct `ChunkPool` in the current scope
error[E0560]: struct `ProbeReport` has no field named `uniform_block`
  --> ium-core/tests/adapter_feishu_contract.rs:56:9
error[E0609]: no field `uniform_block` on type `ProbeReport`
  --> ium-core/tests/adapter_feishu_contract.rs:58:18
error: could not compile `ium-core` (test "adapter_feishu_contract") due to 8 previous errors
```

8 errors: the module `adapter_feishu` does not exist; `ChunkPool` lacks `plan_uniform_tail`/`new_uniform`; `ProbeReport` lacks the `uniform_block` declaration channel (W11 wire fact: even_split shapes are REJECTED 1062009 — the lane prescribes uniform 4 MiB + exact tail, probed live 2026-11-05). The contract file `ium-core/tests/adapter_feishu_contract.rs` pins the public surface; after implementation the assertions move into the module suite and this file is removed (the #52 compile-fail RED pattern).
