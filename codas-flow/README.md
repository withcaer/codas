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

Scenario | Channel | Latency Per Message | Throughput
--|--|--|--
`1:1` | Crossfire (SPSC) | `8ns` | `132M/s`
`Many(1):1` | Flow (Subscriber) | `54ns` | `18M/s`
`Many(1):1` | Crossfire (MPSC) | `38ns` | `26M/s`
`Many(1):1` | Tokio (MPSC) | `67ns` | `15M/s`
`Many(1):Many(1)` | Flow (Stage, Crate Yield) | `24ns` | `42M/s`
`Many(1):Many(1)` | Flow (Stage, Tokio Yield) | `17ns` | `60M/s`
`Many(1):Many(1)` | Tokio (Broadcast) | `27ns` | `36M/s`
`Many(N):1` | Flow (Subscriber) | `89ns` | `11M/s`
`Many(N):1` | Crossfire (MPSC) | `221ns` | `5M/s`
`Many(N):Many(1)` | Flow (Stage, Crate Yield) | `98ns` | `10M/s`
`Many(N):Many(1)` | Flow (Stage, Tokio Yield) | `70ns` | `14M/s`

> Comparative performance of different scenarios, measured on a 13" MacBook Air M3 (2024, 16GB). Exact numbers will vary between platforms.
>
> `Many(1)` scenarios use a multi-producer/consumer capable channel with just one producer and/or consumer.
>
> `Many(N)` scenarios use a multi-producer/consumer capable channel with two or more producers and/or consumers.

## License

Copyright © 2024 - 2026 With Caer, LLC and Alicorn Systems, LLC.

Licensed under the MIT license. Refer to [the license file](../LICENSE.txt) for more info.