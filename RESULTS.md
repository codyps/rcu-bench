# Results

One run, to give an idea of the shape. Rerun on your own hardware before
drawing conclusions; see the caveats in the [README](README.md).

- CPU: AMD Ryzen 9 9955HX (16 cores / 32 threads), threads not pinned
- Linux 6.18, rustc 1.99.0, `N` = 4 readers
- Date: 2026-11-11
- criterion: 20 samples, 2 s measurement per benchmark; the table shows medians
- The groups were not all measured in a single invocation: `read-1`, `read-N`
  and part of `read-1-writer` come from one `cargo bench` call, the rest from a
  second one right after.

Time per read for the `read-*` groups, time per write for the `write-*` groups.

| Subject | read-1 | read-N | read-1-writer | read-N-writer | write-0 | write-N |
|---|---:|---:|---:|---:|---:|---:|
| `baseline-leak` | 0.2 ns | 0.2 ns | 0.7 ns | 0.9 ns | 23.7 ns | 124 ns |
| `local-rcu` | 4.4 ns | 4.5 ns | 29.6 ns | 32.7 ns | 34.2 ns | 368 ns |
| `std-rwlock` | 8.1 ns | 111 ns | 60.2 ns | 245 ns | 8.3 ns | 210 ns |
| `std-rwlock-arc` | 16.3 ns | 284 ns | 122 ns | 781 ns | 20.1 ns | 606 ns |
| `std-mutex-arc` | 16.8 ns | 348 ns | 108 ns | 582 ns | 19.8 ns | 686 ns |
| `tokio-watch` | 8.5 ns | 107 ns | 36.3 ns | 216 ns | 122 ns | 1277 ns |
| `triple_buffer` | 0.4 ns | — | 25.4 ns | — | 5.9 ns | — |
| `left-right` | 12.1 ns | 12.4 ns | 39.0 ns | 32.7 ns | 33.2 ns | 407 ns |
| `swmr-cell` | 4.7 ns | 4.7 ns | 25.0 ns | 28.5 ns | 14.7 ns | 156 ns |
| `swmr-cell-read-preferred` | 1.0 ns | 1.0 ns | 2.9 ns | 3.1 ns | 23.9 ns | 237 ns |
| `read-copy-update` | 1.5 ns | 1.1 ns | 3.3 ns | 4.0 ns | 36.4 ns | 545 ns |
| `arc-swap` | 9.7 ns | 10.0 ns | 45.0 ns | 39.3 ns | 105 ns | 443 ns |
| `arc-swap-load_full` | 17.8 ns | 115 ns | 62.5 ns | 173 ns | 105 ns | 401 ns |
| `arc-swap-cache` | 0.9 ns | 0.9 ns | 11.9 ns | 2.3 ns | 105 ns | 507 ns |
| `hazarc` | 8.3 ns | 8.5 ns | 96.9 ns | 53.0 ns | 26.9 ns | 491 ns |
| `hazarc-cache` | 0.4 ns | 0.4 ns | 9.0 ns | 1.3 ns | 26.9 ns | 709 ns |
| `aarc` | 29.3 ns | 30.0 ns | 49.4 ns | 87.0 ns | 33.1 ns | 189 ns |
| `arcshift` | 0.2 ns | 0.2 ns | 5.0 ns | 2.0 ns | 51.2 ns | 725 ns |
| `arcu` | 16.5 ns | 139 ns | 18.4 ns | 142 ns | 1388 ns | 2842 ns |
| `rcu_cell` | 15.9 ns | 188 ns | 38.0 ns | 299 ns | 15.9 ns | 17.3 µs |
| `basedrop` | 15.8 ns | 234 ns | 51.2 ns | 378 ns | 32.1 ns | 5420 ns |
| `concread-cowcell` | 16.5 ns | 348 ns | 126 ns | 527 ns | 51.0 ns | 1275 ns |
| `seqlock` | 0.5 ns | 0.5 ns | 4.4 ns | 2.2 ns | 8.7 ns | 108 ns |
| `keepcalm-rcu` | 18.4 ns | 193 ns | 278 ns | 613 ns | 21.8 ns | 582 ns |
| `crossbeam-epoch` | 5.9 ns | 9.8 ns | 86.6 ns | 17.4 ns | 36.3 ns | 281 ns |
| `seize` | 5.4 ns | 5.5 ns | 13.5 ns | 13.3 ns | 22.8 ns | 187 ns |
| `sdd` | 5.4 ns | 5.6 ns | 17.3 ns | 17.5 ns | 35.0 ns | 181 ns |
| `haphazard` | 5.3 ns | 19.2 ns | 8.1 ns | 13.6 ns | 96.1 ns | 250 ns |
| `kovan` | 9.7 ns | 10.0 ns | 34.3 ns | 22.9 ns | 37.7 ns | 289 ns |

## Reading the table

- `baseline-leak` never frees anything. It is the floor, not a contender.
- Subjects that write to a shared cacheline on every read (locks, reference
  counts: `std-*`, `tokio-watch`, `arc-swap-load_full`, `arcu`, `rcu_cell`,
  `basedrop`, `concread-cowcell`, `keepcalm-rcu`) are 10 to 20 times slower per
  read with 4 readers than with one. The others stay flat.
- `local-rcu` and `swmr-cell` are within noise of each other on reads (about
  4.5 ns alone, about 30 ns next to a busy writer). Both pay for a full barrier
  on every read.
- `swmr-cell-read-preferred` and `read-copy-update` avoid that barrier and read
  in 1 to 4 ns even next to a busy writer. `read-copy-update` pays with its
  idle-reader behaviour (see `COMPARISONS.md`); `swmr-cell` moves the barrier
  cost to the writer.
- The per-reader caches (`arc-swap-cache`, `hazarc-cache`) and `arcshift` are
  nearly free while nothing changes, and stay cheap under writes.
- `seqlock` is as cheap as it gets, for `Copy` values.
- Writes: `write-N` is dominated by contention with 4 readers spinning on
  reads, which is harsher than any real workload. `rcu_cell` (17 µs) and
  `basedrop` (5 µs) stand out because their writers wait for in-flight readers.
