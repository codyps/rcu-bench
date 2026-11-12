# rcu-bench

Benchmarks comparing [`local-rcu`](https://github.com/codyps/local-rcu) with
other Rust crates that can hold "the latest value" for many readers: RCU
cells, atomically swappable `Arc`s, locks, and cells built on memory
reclamation frameworks.

For what each crate does and how they differ in design, see
[`COMPARISONS.md`](https://github.com/codyps/local-rcu/blob/main/COMPARISONS.md)
in `local-rcu`.

**Results: <https://codyps.github.io/rcu-bench/>** (interactive; measured by CI
on x86_64 and arm64, on every push and weekly).

## Running

```sh
cargo test --release                   # check that every adapter behaves like a cell
cargo bench                            # run everything
cargo bench -- '^read-writer/'         # one group
cargo bench -- '/local-rcu/'           # one subject
cargo run --release --bin report       # markdown tables from target/criterion
```

`RCU_BENCH_MAX_READERS` caps the number of reader threads. By default it is
the number of CPUs, at most 8.

To look at your own run in the results page:

```sh
cargo run --release --bin report -- --json > site/results.json
python3 -m http.server --directory site   # then open http://localhost:8000
```

## What is measured

Every crate is wrapped in a `Subject` (see [`src/lib.rs`](src/lib.rs)), which
reduces it to: create a cell with some reader handles, read the current value,
publish a new value. The value is four `u64`s.

Each group is measured for a range of reader counts `R` (1, 2, 3, 4, 6, 8, ...
up to the cap), so the results show how each subject scales:

| Group | Threads | Reported |
|---|---|---|
| `read` | R readers | time per read, as seen by one reader |
| `read-writer` | R readers + a writer publishing as fast as it can | time per read, as seen by one reader |
| `write` | the writer + R readers reading as fast as they can (R starts at 0) | time per write |

The groups with a writer stop one reader short of the cap, so the writer has a
CPU of its own.

A read includes getting access (guard, lock, `Arc` clone, ...), looking at the
value, and releasing access again. A write includes allocating the new value
and whatever the crate does about the old one.

## Subjects

| Name | Crate | How it is used |
|---|---|---|
| `baseline-leak` | — | `AtomicPtr`, old values are never freed. The floor: one atomic load per read |
| `local-rcu` | `local-rcu` | `Reader::read`, `Writer::write` |
| `local-rcu-cached` | `local-rcu` | `cached::CachedReader::read` (stays in its read section between reads), `Writer::write` |
| `std-rwlock` | std | `Arc<RwLock<T>>`, read in place under the lock |
| `std-rwlock-arc` | std | `RwLock<Arc<T>>`, clone the `Arc` under the lock |
| `std-mutex-arc` | std | `Mutex<Arc<T>>`, clone the `Arc` under the lock |
| `tokio-watch` | `tokio` | `Receiver::borrow`, `Sender::send_replace` |
| `triple_buffer` | `triple_buffer` | `Output::read`, `Input::write`. One reader only, so it has no results above R = 1 |
| `left-right` | `left-right` | the value is the data structure, the only operation replaces it; `append` + `publish` per write |
| `swmr-cell` | `swmr-cell` | `LocalReader::pin`, `SwmrCell::store` |
| `swmr-cell-read-preferred` | `swmr-cell` | the same, built with `read_preferred()` |
| `read-copy-update` | `read-copy-update` | `Reader::read`, `Rcu::write` |
| `arc-swap` | `arc-swap` | `load()` |
| `arc-swap-load_full` | `arc-swap` | `load_full()` |
| `arc-swap-cache` | `arc-swap` | a `Cache` per reader |
| `hazarc` | `hazarc` | `AtomicArc::load` |
| `hazarc-cache` | `hazarc` | a `Cache` per reader |
| `aarc` | `aarc` | `AtomicArc::load`, `store` |
| `arcshift` | `arcshift` | `ArcShift::get` (`&mut`), `update` |
| `arcu` | `arcu` | atomic `Arcu` with the global, thread-local epoch counter pool |
| `rcu_cell` | `rcu_cell` | `RcuCell::read`, `write` |
| `basedrop` | `basedrop` | `SharedCell::get`, `set`; the writer runs the `Collector` after every write |
| `concread-cowcell` | `concread` | `CowCell::read`, write transaction + `commit` |
| `seqlock` | `seqlock` | `read` (copies the value out), `lock_write` |
| `keepcalm-rcu` | `keepcalm` | `SharedMut::new_rcu`, `read`, `set` |
| `crossbeam-epoch` | `crossbeam-epoch` | `Atomic<T>` in the global collector; `pin` per read, `defer_destroy` per write |
| `seize` | `seize` | `AtomicPtr` + a `Collector`; `enter` + `protect` per read, `retire` per write |
| `sdd` | `sdd` | `AtomicOwned`; a `Guard` per read, `swap` per write |
| `haphazard` | `haphazard` | `AtomicPtr`, one `HazardPointer` kept per reader; `retire` per write |
| `kovan` | `kovan` | `Atom::load`, `store` |

Not included:

- `atom_box` 0.3.0 does not compile with current Rust (1.99).
- `rcu_128` needs the unstable `integer_atomics` feature and a target with
  128-bit atomics.
- `rcurs` 0.2.0 crashes when `get` races with `update`.
- `rcu-clean`: old values are only freed by `clean()`, which needs every
  handle to be idle. With continuous reads and writes the chain of old
  versions grows without bound and overflows the stack when it is dropped.
- `urcu` and `urcu2` need liburcu installed.
- `tokio_rcu` needs `tokio_unstable` and its own runtime setup.
- `ebr` has no pointer type to build a cell from.
- `std::sync::mpsc` and `tokio::sync::broadcast` are queues, not cells.

## Caveats

- These are micro-benchmarks of the cell alone. In a real program the read is
  usually a small part of the work, and cache effects differ.
- The subjects do not offer the same guarantees, so a faster number is not
  automatically "better". Some hand out borrows and others owned `Arc`s; some
  return old values to the writer and others drop them on a reader thread;
  some let a slow reader block the writer. The adapters use each crate the way
  its documentation suggests for this job, not the way that happens to be
  fastest.
- The writer in `read-writer` publishes continuously, and the readers in
  `write` read continuously. Both are far from a typical "read mostly" load.
- Readers here release access right after every read. Nothing measures a
  reader that holds a guard for a long time.
- All threads are unpinned and results depend on the machine. Compare columns
  within one run only.

## CI

[`.github/workflows/bench.yml`](.github/workflows/bench.yml) runs the smoke
tests and the benchmarks on GitHub-hosted x86_64 and arm64 runners, merges the
runs with [`scripts/merge_results.py`](scripts/merge_results.py), and publishes
[`site/`](site) to GitHub Pages. The page has a machine switch.

Those runners have 4 CPUs and noisy neighbours, so the sweep is short (up to 4
readers alone, 3 beside a writer) and the numbers are rougher than a run on a
quiet machine.

To add a self-hosted machine, register a runner for this repository and set the
repository variable `EXTRA_RUNNER` to its `runs-on` label (and optionally
`EXTRA_RUNNER_NAME` to the name the page should show). The workflow only runs
on pushes to `main`, on a schedule and by hand, never for pull requests.

## Adding a subject

1. Add the crate to `Cargo.toml` and a module in `src/subjects/` implementing
   `Subject` (name, kind, a one-line description, and the three operations).
2. List it in `for_each_subject!` in `src/lib.rs`.
3. `cargo test --release` checks it sequentially and concurrently.
