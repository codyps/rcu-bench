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
the number of CPUs, at most 8. `RCU_BENCH_OVERSUBSCRIBED` is the number of
readers in the extra, oversubscribed run of `read-writer` and `write`: twice
the number of CPUs by default, 0 for none. `RCU_BENCH_MAX_IDLE` caps the number of idle
readers in `write-idle`, 64 by default. `RCU_BENCH_WRITE_HZ` sets the writes
per second in `read-paced`, 1,000,000 by default. `RCU_BENCH_HOLD_US` sets the
microseconds a reader stays in each read in `write-held`, 10 by default; set
it for `report` as well, which names the group after it.

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
| `read-paced` | R readers + a writer publishing at a fixed rate | time per read, as seen by one reader |
| `write` | the writer + R readers reading as fast as they can (R starts at 0) | time per write |
| `write-idle` | the writer + R idle readers (R = 1, 4, 16, 64) | time per write |
| `write-held` | the writer + R readers that stay in every read for a fixed time | time per write |

The groups with a writer stop one reader short of the cap, so the writer has a
CPU of its own.

`read-writer` and `write` are measured once more with twice as many readers
as there are CPUs. The threads then take turns, and the scheduler takes a
reader or the writer off its CPU in the middle of a read or a write. A thread
that waits for one that is not running (for a lock it holds, or for it to
leave its read) waits for a whole time slice. Reads are timed by the clock, so
every subject's reads take longer there, by about the share of a CPU a reader
still gets; compare the subjects with each other, not with the other columns.
The report and the results page say which columns these are.

Some subjects get nowhere at all with that many threads: a writer that needs a
moment with no reader in its way may never find one. So before an
oversubscribed benchmark runs, the threads it would time get 2 seconds for
1,000 operations. If that is not enough the benchmark is not run, and the
report and the page show it as stalled, with how far it got.

Criterion times many operations in a row and reports the time per operation,
which hides the slow ones. `read-writer` and `write` are therefore run once
more after criterion is done, with every read (or write) timed on its own, for
half a second (`RCU_BENCH_TAIL_MS`, 0 for not at all). The report and the page
show the time that 99% and 99.9% of them stayed within, and the slowest one.
Each of those times includes one reading of the clock (the end of one
operation is the start of the next), a few tens of nanoseconds: for the
fastest subjects that is most of it, and timed readers also read less often
than the ones criterion measures. These numbers are for the stalls of
microseconds and more. The loop is closed, so one stall counts once, however
many operations would have fallen into it; and the slowest operation of half a
second on a shared machine is largely luck.

In `read-writer` the writers do not all do the same work: a subject with cheap
writes gets more of them, and its readers pay for every one (a cache miss on
the new value, at least). Its reads then look slower than those of a subject
whose writer is slow. `read-paced` takes that out by giving every subject the
same number of writes per second: 1,000,000, which the readers notice but the
slowest writers do not keep up with. (At 50,000 writes per second the results
were those of `read`, so that rate is no longer measured.) The writer spins
between writes instead of sleeping, so that it stays on its CPU and on time,
and catches up without a pause after falling behind. A writer that cannot
reach the rate at all writes as fast as it can.

Every read group with a writer records the writes per second there really
were (`writes.json`, next to criterion's results). The report and the results
page show them, name the paced group after the rate it ran at, and mark
with † the results where the writer reached less than 90% of it: those are for
fewer writes than the others had.

An idle reader in `write-idle` has read once, on a thread of its own, and that
thread then sleeps without giving up its handle. It uses no CPU, so R is not
tied to the number of CPUs (`RCU_BENCH_MAX_IDLE` caps it, 64 by default). What
this shows is how the cost of a write grows with the number of readers that
merely exist: a writer that looks at every reader, or at every thread known to
a collector, pays for each of them.

A reader in `write-held` does not give up its access to the value (guard,
lock, borrow, `Arc`, ...) right after looking at it: it spins for 10 µs with
the access kept, gives it up, and reads again at once. So every reader is
inside a read nearly all of the time, and the time per write shows what that
costs the writer: one that waits for the readers to leave pays for the time
they stay (and for being woken up, if it sleeps meanwhile), one that only sets
the old value aside pays nothing extra, and a reader that holds an `Arc` or a
copy does not hold the writer up at all. A lock that lets a waiting writer in
ahead of the readers can look cheap here with several readers: the writer
then gets many writes done while the readers are still being woken up. What
the old values that pile up meanwhile cost in memory is not measured.

A read includes getting access (guard, lock, `Arc` clone, ...), looking at the
value, and releasing access again. A write includes allocating the new value
and whatever the crate does about the old one.

## Subjects

| Name | Crate | How it is used |
|---|---|---|
| `baseline-leak` | — | `AtomicPtr`, old values are never freed. The floor: one atomic load per read |
| `local-rcu` | `local-rcu` | `Reader::read`, `Writer::write` |
| `local-rcu-cached` | `local-rcu` | `cached::CachedReader::read` (stays in its read section between reads), `Writer::write` |
| `local-rcu-batched` | `local-rcu` | `Reader::read`, `Writer::write_nosync` (a pointer store that leaves the readers alone); old values are scanned for and collected in batches (`try_sync`) once every 128 writes, the way an epoch collector does |
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
| `seqlock` | `seqlock` | `read` (copies the value out), `lock_write`. `Copy` values only, marked "Copy" on the results page |
| `keepcalm-rcu` | `keepcalm` | `SharedMut::new_rcu`, `read`, `set` |
| `crossbeam-epoch` | `crossbeam-epoch` | `Atomic<T>` in the global collector; `pin` per read, `defer_destroy` per write |
| `seize` | `seize` | `AtomicPtr` + a `Collector`; `enter` + `protect` per read, `retire` per write |
| `sdd` | `sdd` | `AtomicOwned`; a `Guard` per read, a `Guard` and a `swap` per write (a thread only frees what it retired when it creates guards) |
| `haphazard` | `haphazard` | `AtomicPtr`, one `HazardPointer` kept per reader; `retire` per write |
| `kovan` | `kovan` | `Atom::load`, `store` |

Not included:

- `kovan`'s lower level API (`Atomic`, `pin`, `retire`, as in its own
  benchmarks) was measured next to `Atom` and gave the same results.
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
- Except in `write-held`, readers release access right after every read. And
  `write-held` only has readers that hold on for microseconds: nothing
  measures a reader that keeps a guard for seconds, or that never lets go.
- `report` does check what a reader that has gone *idle* holds back: the
  writer publishes a value, one reader reads it and then does nothing, and
  the writer publishes 100,000 more. It then counts the allocations made by
  those writes that are still alive, and looks at whether the allocations of
  the value the reader read are (both again with the reader gone before the
  writes, as a control). Next to the allocations it counts the old values
  themselves, by searching the memory of every live allocation for them
  (freed memory is wiped): a subject may need several allocations to keep
  one value, or none for a value stored in place. Every one of these
  measurements runs in a process of its own, because several subjects keep
  a collector for the whole process, with whatever the previous measurement
  left in it. On the results page ◆ marks subjects where the idle
  reader keeps every value since its read alive (`local-rcu-cached`,
  `read-copy-update`), and ◇ those where it keeps only the value it read
  (`arc-swap-cache`, `hazarc-cache`, `arcshift`). The markdown report lists
  the same under "Idle readers".
- `report` also checks for batched reclamation: with no reader at all, the
  writer publishes 100,000 values and stops. Subjects that still have a batch
  of old values allocated then (8 or more, short of all of them) are marked ≡
  on the page: `local-rcu-batched`, `swmr-cell`, `crossbeam-epoch`,
  `sdd`, `haphazard`, `kovan`. The count is repeated after 200 ms of nothing
  happening, to tell apart subjects where a background task does the freeing
  (↻). None of the subjects has one as used here.
- All threads are unpinned and results depend on the machine. Compare columns
  within one run only.

## CI

[`.github/workflows/bench.yml`](.github/workflows/bench.yml) runs the smoke
tests and the benchmarks on GitHub-hosted x86_64 and arm64 runners, merges the
runs with [`scripts/merge_results.py`](scripts/merge_results.py), and publishes
[`site/`](site) to GitHub Pages. The page has a machine switch.

Those runners have 4 CPUs and noisy neighbours, so the sweep is short (up to 4
readers alone, 3 beside a writer, and 8 for the oversubscribed run) and the numbers are rougher than a run on a
quiet machine.

To add a self-hosted machine, register a runner for this repository and set the
repository variable `EXTRA_RUNNER` to its `runs-on` label (and optionally
`EXTRA_RUNNER_NAME` to the name the page should show). The workflow only runs
on pushes to `main`, on a schedule and by hand, never for pull requests.

## Adding a subject

1. Add the crate to `Cargo.toml` and a module in `src/subjects/` implementing
   `Subject` (name, kind, a one-line description, and the three operations:
   `new`, `read_with` and `write`).
2. List it in `for_each_subject!` in `src/lib.rs`.
   Set `COPY_ONLY` if the crate can only hold `Copy` values.
3. `cargo test --release` checks it sequentially and concurrently.
