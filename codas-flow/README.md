[![`codas-flow` on crates.io](https://img.shields.io/crates/v/codas-flow)](https://crates.io/crates/codas-flow)
[![`codas-flow` on docs.rs](https://img.shields.io/docsrs/codas-flow)](https://docs.rs/codas-flow/)
[![`codas-flow` is MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](../LICENSE.txt)

Low-latency, high-throughput bounded queues ("data flows")
for (a)synchronous and event-driven systems, inspired by the
[LMAX Disruptor](https://github.com/LMAX-Exchange/disruptor)
and built for [`codas`](https://crates.io/crates/codas).

## What's Here

This crate provides the `flow` data structure: A
["ring" buffer](https://en.wikipedia.org/wiki/Circular_buffer)
which concurrent, (a)synchronous tasks can publish data to 
and receive data from.

`flow`s work kind of like the `broadcast` channels in
[Tokio](https://docs.rs/tokio/latest/tokio/sync/broadcast/fn.channel.html),
with some key differences:

1. **Zero-Copy Multicast Reads**: Every data published to a
   flow is immediately available to _each_ subscriber, in
   parallel, with no copies or cloning.

2. **Lock-Free** by default: No locks or mutexes are used
   when publishing _or_ receiving data from the `flow` on
   [supported targets](#lock-free-targets).

3. **Broad Compatibility**:
  - `no-std` by default.
  - `async` _and_ synchronous APIs.
  - `async` functionality doesn't depend on a specific
    runtime or framework (not even `futures`!).

`flow`s work wherever channels or queues would work, but they're built
specifically for systems that need the same data processed concurrently
(or in parallel) by multiple tasks.

## Examples

Flows are created with `Flow::new`, which returns a
tuple of `(flow, subscribers)`:

```rust
use codas_flow::*;

// Create a flow with a capacity of 32 strings,
// and one subscriber.
let (mut flow, [mut sub]) = Flow::<String>::new(32);

// Publish "Hello!" to the next data sequence in the flow.
let seq = flow.try_next().unwrap();
seq.publish("Hello!".to_string());

// Receive the next published data sequence from the flow.
let seq = sub.try_next().unwrap();
assert_eq!("Hello!", *seq);
```

Data is published _into_ a `flow` via `Flow::try_next`
(or `await Flow::next`), which returns an `UnpublishedData`
reference. Once this reference is published (via `UnpublishedData::publish`),
or dropped, it becomes receivable by every subscriber.

Data is received _from_ a `flow` via `FlowSubscriber::try_next`
(or `await FlowSubscriber::next`), which returns a `PublishedData`
reference.

### Subscribers

Using
[slice patterns](https://doc.rust-lang.org/reference/patterns.html#slice-patterns),
any number of subscribers can be returned by `Flow::new`:

```rust
use codas_flow::*;

// Create a flow with a capacity of 32 strings,
// and 2 subscribers.
let (mut flow, [mut sub_a, mut sub_b]) = Flow::<String>::new(32);
```

New subscribers _cannot_ be added to an active flow. To overcome
this challenge, any subscriber can be wrapped in a **Stage**.

### Stages

A stage is a dynamic group of data processors
that share a single subscriber:

```rust
# use core::sync::atomic::Ordering;
# use portable_atomic::AtomicU64;
# use portable_atomic_util::Arc;
use codas_flow::{*, stage::*};

// Create a flow.
let (mut flow, [mut sub]) = Flow::<String>::new(32);

// Wrap the subscriber in a processing stage.
let mut stage = Stage::from(sub);

// Add a data processor to the stage; an indefinite 
// number of processors can be added to a stage, even
// while the flow is active.
let calls = Arc::new(AtomicU64::new(0));
let closure_calls = calls.clone();
stage.add_proc(move |proc: &mut Proc, data: &String| {
   assert_eq!("Hello!", *data);
   closure_calls.add(1, Ordering::SeqCst);
});

// Publish "Hello!" to the next data sequence in the flow.
let seq = flow.try_next().unwrap();
seq.publish("Hello!".to_string());

// Run processors for a set of data in the flow.
stage.proc();
assert_eq!(1, calls.load(Ordering::SeqCst));
```

Stages only receive data from the flow when one of the
`Stage::proc*` functions is invoked; refer to the `Stage`
docs for more information.

## Lock-Free Targets

This crate uses `AtomicU64` to coordinate `flow` access
without locks. This type is lock-free [_where possible_](https://doc.rust-lang.org/std/sync/atomic/#portability), but may use locks on some platforms or compile targets.

This section contains a list of the primary targets supported
by this crate, along with their support for lock-free behavior.

Target | Lock-Free?
-------|-----------
`aarch64-unknown-linux-gnu` (64-Bit Linux ARM) | Yes
`aarch64-apple-darwin` (64-Bit MacOS ARM) | Yes
`x86_64-unknown-linux-gnu` (64-Bit Linux) | Yes
`x86_64-apple-darwin` (64-Bit MacOS) | Yes
`x86_64-pc-windows-gnu` (64-Bit Windows) | Yes
`wasm32-unknown-unknown` (WASM) | Yes<sup>1</sup>
`armv7-unknown-linux-gnueabihf` (ARM Cortex A7 and A8) | No<sup>2</sup>
`riscv32i-unknown-none-elf` (ESP 32) | [No](https://github.com/espressif/esp-idf/commit/d4f2e03e4aa58a395c7479aa7f3b39eceddccf9f#diff-595e87f9629868ce4ba58aa40914f008cad51b6cf963daa08d38e5c827bbdf14R63)

> **<sup>1</sup>** WASM targets don't technically support atomic instructions. However, because WASM code is executed in a single-thread, regular variables are simply substituted for their atomic counterparts, enabling full lock-free support.
>
> **<sup>2</sup>** Confirmation required; a safe assumption is that 32-bit targets don't support atomic operations on 64-bit values.

## Relative Performance [("Benchmarks")](benches/channels.rs)

First, a caveat: Benchmarks are quite noisy, and shouldn't be used as absolute references--particularly benchmarks from different platforms. Instead, these benchmarks should be used to understand the relevant performance of different scenarios and frameworks on the _same_ platform.

Each benchmark table  contains a `Scenario` column, which describes the number of producers and consumers in the test:

- `Many(1)` scenarios use a multi-producer/consumer capable channel with just one producer and/or consumer. 

- `Many(N)` scenarios use a multi-producer/consumer capable channel with two or more producers and/or consumers.

Benchmarks on a `13" MacBook Air M3 (2024, 16GB)`:

<details>

Scenario | Channel | Latency Per Message | Throughput
--|--|--|--
`1:1` | Crossfire (SPSC) | `7ns` | `140M/s`
`1:1` | Disruptor (Single Producer) | `7ns` | `143M/s`
`Many(1):1` | Flow (Subscriber) | `55ns` | `18M/s`
`Many(1):1` | Crossfire (MPSC) | `38ns` | `27M/s`
`Many(1):1` | Disruptor (Multi Producer) | `20ns` | `50M/s`
`Many(1):1` | Tokio (MPSC) | `66ns` | `15M/s`
`Many(1):Many(1)` | Flow (Stage, Crate Yield) | `23ns` | `43M/s`
`Many(1):Many(1)` | Flow (Stage, Tokio Yield) | `16ns` | `64M/s`
`Many(1):Many(1)` | Tokio (Broadcast) | `27ns` | `37M/s`
`Many(N):1` | Flow (Subscriber) | `91ns` | `11M/s`
`Many(N):1` | Crossfire (MPSC) | `32ns` | `31M/s`
`Many(N):1` | Disruptor (Multi Producer) | `538ns` | `2M/s`
`Many(N):Many(1)` | Flow (Stage, Crate Yield) | `96ns` | `10M/s`
`Many(N):Many(1)` | Flow (Stage, Tokio Yield) | `70ns` | `14M/s`

</details>
&nbsp;

Benchmarks on a a `Hetzner CCX23 AMD EPYC, 4 dedicated vCPUs, 16GB`:

<details>

Scenario | Channel | Latency Per Message | Throughput
--|--|--|--
`1:1` | Crossfire (SPSC) | `15ns` | `68M/s`
`1:1` | Disruptor (Single Producer) | `9ns` | `114M/s`
`Many(1):1` | Flow (Subscriber) | `69ns` | `14M/s`
`Many(1):1` | Crossfire (MPSC) | `27ns` | `37M/s`
`Many(1):1` | Disruptor (Multi Producer) | `42ns` | `24M/s`
`Many(1):1` | Tokio (MPSC) | `68ns` | `15M/s`
`Many(1):Many(1)` | Flow (Stage, Crate Yield) | `50ns` | `20M/s`
`Many(1):Many(1)` | Flow (Stage, Tokio Yield) | `35ns` | `28M/s`
`Many(1):Many(1)` | Tokio (Broadcast) | `41ns` | `24M/s`
`Many(N):1` | Flow (Subscriber) | `92ns` | `11M/s`
`Many(N):1` | Crossfire (MPSC) | `33ns` | `30M/s`
`Many(N):1` | Disruptor (Multi Producer) | `242ns` | `4M/s`
`Many(N):Many(1)` | Flow (Stage, Crate Yield) | `128ns` | `8M/s`
`Many(N):Many(1)` | Flow (Stage, Tokio Yield) | `80ns` | `12M/s`

</details>

## License

Copyright © 2024 - 2026 With Caer, LLC and Alicorn Systems, LLC.

Licensed under the MIT license. Refer to [the license file](../LICENSE.txt) for more info.