use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use codas_flow::{
    stage::{Proc, Stage},
    Flow,
};
use criterion::{criterion_group, criterion_main, Criterion};
use crossfire::mpsc as crossfire_mpsc;
use crossfire::spsc as crossfire_spsc;
use disruptor::{build_multi_producer, build_single_producer, BusySpin, Producer, Sequence};
use tokio::sync::{broadcast, mpsc};

const BUFFER_SIZE: usize = 1024;
const BACKGROUND_PRODUCERS: usize = 3;

fn channels(c: &mut Criterion) {
    let mut group = c.benchmark_group("Channels");
    group.throughput(criterion::Throughput::Elements(1));

    group.bench_function("Many(1):1 Flow (Subscriber); Move->Read", |b| {
        let i = RefCell::new(0);
        let (pubs, [mut subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);
        let pubs = RefCell::new(pubs);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            let mut next_i = 0;

            loop {
                let data = subs.next().await.expect("value");
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            }
        });

        // Publish lots of events.
        b.to_async(runtime).iter(|| async {
            let mut pubs = pubs.borrow_mut();
            let mut next = pubs.next().await.expect("next");
            let mut i = i.borrow_mut();
            next.value = *i;
            drop(next);
            *i += 1;
        });
    });

    group.bench_function(
        "Many(1):Many(1) Flow (Stage); Move->Read (Crate Yield)",
        |b| {
            let i = RefCell::new(0);
            let (pubs, [subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);
            let pubs = RefCell::new(pubs);

            // Prepare event handler.
            let mut stage = Stage::from(subs);
            let mut next_i = 0;
            stage.add_proc(move |_: &mut Proc, data: &TestStruct| {
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            });

            // Spawn event handler in a loop.
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.spawn(stage.proc_loop());

            // Publish lots of events.
            b.to_async(runtime).iter(|| async {
                let mut pubs = pubs.borrow_mut();
                let mut next = pubs.next().await.expect("next");
                let mut i = i.borrow_mut();
                next.value = *i;
                drop(next);
                *i += 1;
            });
        },
    );

    group.bench_function(
        "Many(1):Many(1) Flow (Stage); Move->Read (Tokio Yield)",
        |b| {
            let i = RefCell::new(0);
            let (pubs, [subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);
            let pubs = RefCell::new(pubs);

            // Prepare event handler.
            let mut stage = Stage::from(subs);
            let mut next_i = 0;
            stage.add_proc(move |_: &mut Proc, data: &TestStruct| {
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            });

            // Spawn event handler in a loop.
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.spawn(stage.proc_loop_with_waiter(tokio::task::yield_now));

            // Publish lots of events.
            b.to_async(runtime).iter(|| async {
                let mut pubs = pubs.borrow_mut();
                let mut next = pubs.next().await.expect("next");
                let mut i = i.borrow_mut();
                next.value = *i;
                drop(next);
                *i += 1;
            });
        },
    );

    group.bench_function("Many(1):1 Tokio (MPSC); Move->Take", |b| {
        let i = RefCell::new(0);
        let (tx, mut rx) = mpsc::channel::<TestStruct>(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            let mut next_i = 0;

            loop {
                let data = rx.recv().await.expect("value");
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            }
        });

        // Publish lots of events.
        b.to_async(runtime).iter(|| async {
            tx.send(TestStruct { value: *i.borrow() }).await.unwrap();
            *i.borrow_mut() += 1;
        });
    });

    group.bench_function("Many(1):1 Crossfire (MPSC); Move->Take", |b| {
        let i = RefCell::new(0);
        let (tx, rx) = crossfire_mpsc::bounded_async::<TestStruct>(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            let mut next_i = 0;

            loop {
                let data = rx.recv().await.expect("value");
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            }
        });

        // Publish lots of events.
        b.to_async(runtime).iter(|| async {
            tx.send(TestStruct { value: *i.borrow() }).await.unwrap();
            *i.borrow_mut() += 1;
        });
    });

    group.bench_function("1:1 Crossfire (SPSC); Move->Take", |b| {
        let i = RefCell::new(0);
        let (tx, rx) = crossfire_spsc::bounded_async::<TestStruct>(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            let mut next_i = 0;

            loop {
                let data = rx.recv().await.expect("value");
                assert_eq!(next_i, data.value as u64);
                next_i += 1;
            }
        });

        // Publish lots of events.
        b.to_async(runtime).iter(|| async {
            tx.send(TestStruct { value: *i.borrow() }).await.unwrap();
            *i.borrow_mut() += 1;
        });
    });

    group.bench_function("1:1 Disruptor (Single Producer); Mutate->Read", |b| {
        let mut next_i = 0u64;
        let processor = move |e: &TestStruct, _sequence: Sequence, _end_of_batch: bool| {
            assert_eq!(next_i, e.value as u64);
            next_i += 1;
        };

        let mut producer = build_single_producer(BUFFER_SIZE, TestStruct::default, BusySpin)
            .handle_events_with(processor)
            .build();

        let mut i = 0i64;
        b.iter(|| {
            let val = i;
            producer.publish(|e| {
                e.value = val;
            });
            i += 1;
        });
    });

    group.bench_function("Many(1):1 Disruptor (Multi Producer); Mutate->Read", |b| {
        let mut next_i = 0u64;
        let processor = move |e: &TestStruct, _sequence: Sequence, _end_of_batch: bool| {
            assert_eq!(next_i, e.value as u64);
            next_i += 1;
        };

        let mut producer = build_multi_producer(BUFFER_SIZE, TestStruct::default, BusySpin)
            .handle_events_with(processor)
            .build();

        let mut i = 0i64;
        b.iter(|| {
            let val = i;
            producer.publish(|e| {
                e.value = val;
            });
            i += 1;
        });
    });

    group.bench_function("Many(N):1 Disruptor (Multi Producer); Mutate->Read", |b| {
        let processor = |e: &TestStruct, _sequence: Sequence, _end_of_batch: bool| {
            std::hint::black_box(e.value);
        };

        let mut producer = build_multi_producer(BUFFER_SIZE, TestStruct::default, BusySpin)
            .handle_events_with(processor)
            .build();

        // Spawn background producers on dedicated threads.
        let running = Arc::new(AtomicBool::new(true));
        for _ in 0..BACKGROUND_PRODUCERS {
            let mut bg_producer = producer.clone();
            let running = running.clone();
            std::thread::spawn(move || {
                let mut val = 0i64;
                while running.load(Ordering::Relaxed) {
                    bg_producer.publish(|e| {
                        e.value = val;
                    });
                    val += 1;
                }
            });
        }

        // Publish from the benchmark thread under contention.
        let mut i = 0i64;
        b.iter(|| {
            let val = i;
            producer.publish(|e| {
                e.value = val;
            });
            i += 1;
        });

        running.store(false, Ordering::Relaxed);
    });

    group.bench_function("Many(N):1 Crossfire (MPSC); Move->Take", |b| {
        let (tx, rx) = crossfire_mpsc::bounded_async::<TestStruct>(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            loop {
                let _data = rx.recv().await.expect("value");
            }
        });

        // Spawn background producers.
        for _ in 0..BACKGROUND_PRODUCERS {
            let tx = tx.clone();
            let counter = Arc::new(AtomicI64::new(0));
            let c = counter.clone();
            runtime.spawn(async move {
                loop {
                    let val = c.fetch_add(1, Ordering::Relaxed);
                    if tx.send(TestStruct { value: val }).await.is_err() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            });
        }

        // Publish from the benchmark thread under contention.
        let i = RefCell::new(0);
        b.to_async(runtime).iter(|| async {
            tx.send(TestStruct { value: *i.borrow() }).await.unwrap();
            *i.borrow_mut() += 1;
        });
    });

    group.bench_function("Many(N):1 Flow (Subscriber); Move->Read", |b| {
        let (pubs, [mut subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            loop {
                let _data = subs.next().await.expect("value");
            }
        });

        // Spawn background producers.
        for _ in 0..BACKGROUND_PRODUCERS {
            let mut pubs = pubs.clone();
            let counter = Arc::new(AtomicI64::new(0));
            let c = counter.clone();
            runtime.spawn(async move {
                loop {
                    match pubs.next().await {
                        Ok(mut next) => {
                            next.value = c.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => break,
                    }
                    tokio::task::yield_now().await;
                }
            });
        }

        // Publish from the benchmark thread under contention.
        let pubs = RefCell::new(pubs);
        let i = RefCell::new(0);
        b.to_async(runtime).iter(|| async {
            let mut pubs = pubs.borrow_mut();
            let mut next = pubs.next().await.expect("next");
            let mut i = i.borrow_mut();
            next.value = *i;
            drop(next);
            *i += 1;
        });
    });

    group.bench_function(
        "Many(N):Many(1) Flow (Stage); Move->Read (Crate Yield)",
        |b| {
            let (pubs, [subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);

            // Prepare event handler.
            let mut stage = Stage::from(subs);
            stage.add_proc(move |_: &mut Proc, data: &TestStruct| {
                std::hint::black_box(data.value);
            });

            // Spawn event handler in a loop.
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.spawn(stage.proc_loop());

            // Spawn background producers.
            for _ in 0..BACKGROUND_PRODUCERS {
                let mut pubs = pubs.clone();
                let counter = Arc::new(AtomicI64::new(0));
                let c = counter.clone();
                runtime.spawn(async move {
                    loop {
                        match pubs.next().await {
                            Ok(mut next) => {
                                next.value = c.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => break,
                        }
                        tokio::task::yield_now().await;
                    }
                });
            }

            // Publish from the benchmark thread under contention.
            let pubs = RefCell::new(pubs);
            let i = RefCell::new(0);
            b.to_async(runtime).iter(|| async {
                let mut pubs = pubs.borrow_mut();
                let mut next = pubs.next().await.expect("next");
                let mut i = i.borrow_mut();
                next.value = *i;
                drop(next);
                *i += 1;
            });
        },
    );

    group.bench_function(
        "Many(N):Many(1) Flow (Stage); Move->Read (Tokio Yield)",
        |b| {
            let (pubs, [subs]) = Flow::<TestStruct>::new(BUFFER_SIZE);

            // Prepare event handler.
            let mut stage = Stage::from(subs);
            stage.add_proc(move |_: &mut Proc, data: &TestStruct| {
                std::hint::black_box(data.value);
            });

            // Spawn event handler in a loop.
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.spawn(stage.proc_loop_with_waiter(tokio::task::yield_now));

            // Spawn background producers.
            for _ in 0..BACKGROUND_PRODUCERS {
                let mut pubs = pubs.clone();
                let counter = Arc::new(AtomicI64::new(0));
                let c = counter.clone();
                runtime.spawn(async move {
                    loop {
                        match pubs.next().await {
                            Ok(mut next) => {
                                next.value = c.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => break,
                        }
                        tokio::task::yield_now().await;
                    }
                });
            }

            // Publish from the benchmark thread under contention.
            let pubs = RefCell::new(pubs);
            let i = RefCell::new(0);
            b.to_async(runtime).iter(|| async {
                let mut pubs = pubs.borrow_mut();
                let mut next = pubs.next().await.expect("next");
                let mut i = i.borrow_mut();
                next.value = *i;
                drop(next);
                *i += 1;
            });
        },
    );

    group.bench_function("Many(1):Many(1) Tokio (Broadcast); Move->Clone", |b| {
        let i = RefCell::new(0);
        let (tx, mut rx) = broadcast::channel::<TestStruct>(BUFFER_SIZE);

        // Spawn event handler in a loop.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.spawn(async move {
            let mut next_i = 0;

            loop {
                match rx.recv().await {
                    Ok(data) => {
                        assert_eq!(next_i, data.value as u64);
                        next_i += 1;
                    }
                    Err(broadcast::error::RecvError::Lagged(lag)) => next_i += lag,
                    Err(..) => unimplemented!(),
                }
            }
        });

        // Publish lots of events.
        b.to_async(runtime).iter(|| async {
            let mut i = i.borrow_mut();
            let _ = tx.send(TestStruct { value: *i }).unwrap();
            *i += 1;
        });
    });
}

// Create a new group named `benches` and
// run it with all benchmark methods.
criterion_group!(benches, channels);
criterion_main!(benches);

/// Simplistic test data structure for [`channels`].
#[derive(Debug, Default, Clone)]
struct TestStruct {
    value: i64,
}
