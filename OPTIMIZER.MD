# Exact optimizer in Rust

## Contract

Given the user's fixed subjects and merged API/manual time choices, return one deterministic optimum with selected choice IDs, or explicit infeasibility, plus unresolved-time notices. Claim optimality only over known, supported options. Retain grouped members for later section/room selection.

The public library entry point is `optimizer::solve(&[Requirement], Option<&AtomicBool>) -> anyhow::Result<Solution>`. Input errors remain errors, not infeasibility. A cancellation flag produces `cancelled` with no optimality claim. The UI runs this pure computation on a worker thread. No Python process or external solver is involved.

For the MVP's full-term weekly model, minimize lexicographically:

```text
(number of occupied days of the week, total daily gap minutes)
gap(day) = last end - first start - occupied meeting minutes
```

Count weekends if used. Empty days contribute zero; ignore time outside the first/last meeting. Fewer days always outrank fewer gaps. Intervals are `[start,end)`, so adjacent meetings do not conflict. Partial-term optimization remains outside this initial model; preserve and disclose unsupported dates as specified in README.MD.

## Baseline

1. Deduplicate interchangeable complete time patterns within each required component.
2. Keep components as separate choice groups. Avoid precomputing cross-component bundles. Omit components without known times from search and list them as unresolved. An empty domain caused by conflicts is infeasible.
3. Encode complete options as weekly bitsets. Use one bit per minute across seven days: 10,080 bits in `[u64; 158]`, 1,264 bytes per bitset. Keep unused tail bits zero. Preserve exact manual times. Conflict checks scan corresponding words, avoiding arbitrary-precision or compressed-bitset dependencies.
4. Process fixed/few-option groups first. Backtrack on overlaps or known pairing incompatibilities.
5. Score complete feasible assignments as they are generated. Retain only the best, using stable order for ties.

```text
search(remaining_groups, occupancy, selected):
    if none remain: compare score and retain best assignment; return
    choose the next group
    for option in group:
        if occupancy & option.bits: continue
        if incompatible with selected: continue
        search(other_groups, occupancy | option.bits, selected + option)
```

Exactness: every feasible assignment is examined unless a proven-safe bound excludes it. Existing conflicts persist as meetings are added. Later meetings can fill gaps, making current gap size unsafe for pruning. Compare `Score { occupied_days, gap_minutes }` lexicographically. Define stable requirement/option ordering and tie behavior rather than relying on hash iteration.

Grouping must account for both outbound and inbound option incompatibilities. Same-time options are not interchangeable if any selected option can distinguish between their IDs under pairing restrictions. Preserve actual members and their original labels/rooms. Dates and explicit unsupported reasons stay visible, but date-limited options do not enter the full-term search. Unknown-only components are reported as unresolved, while a previously nonempty supported domain exhausted by conflicts is infeasible.

## Optional acceleration, only after measurement

- Forward-check other groups after each choice; abandon empty domains. Dynamic MRV selects the smallest surviving domain. Precomputed compatibility bitsets can accelerate this.
- Try attainable day subsets in increasing size, including zero/one-day cases. Optimize gaps across all feasible subsets at the minimum day count. There are 32 subsets for five days, 128 for seven.
- A safe gap lower bound counts empty minutes inside existing daily spans that NO remaining option can fill. Prune only when it cannot improve the incumbent at the relevant day count.

Earlier small research experiments favored simple bitset search in three of four Hydrant cases; both versions matched exhaustive scores. Those experiments are not a Rust/Python benchmark or proof of end-user performance. No universally fastest algorithm was established. Do not add CP-SAT, MILP, genetic search, or complex DP by default. Measure actual release builds on real selected-subject datasets before introducing acceleration.

## Tests

Compare scores AND returned assignments against an independent Cartesian-product oracle. Cover empty/infeasible inputs, adjacency/overlap, atomic multi-day bundles, deduplication, varying durations, minute-resolution manual entries, deterministic ties, and pairing restrictions. Explicitly test day priority over gaps, later gap filling, and better schedules in another equally small day subset. Same-time member switching must preserve times. Keep missing/TBA and unsupported-date components visible. Report feasibility only for known, supported meetings.

Use interval comparisons and independently computed gaps in the oracle, not the production bitset/scoring helpers. Test asymmetric compatibility before grouping, invalid interval bounds, week/word boundaries, and cancellation. Run these against the actual library. Supplement with live-catalog CLI/TUI acceptance, because a scratch harness cannot establish that the application, data adapter, or calendar export is correct.
