#![cfg_attr(all(not(test)), no_std)]
// Use the README file as the root-level
// docs for this library.
#![doc = include_str!("../README.md")]

extern crate alloc;

use core::{
    cell::UnsafeCell,
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut, Range},
    pin::Pin,
    sync::atomic::Ordering,
    task::{Context, Poll},
};

use alloc::{boxed::Box, vec::Vec};
use portable_atomic::AtomicU64;
use portable_atomic_util::{Arc, Weak};
use snafu::Snafu;

pub mod async_support;
pub mod stage;

/// Bounded queue for publishing and receiving
/// data from (a)synchronous tasks.
///
/// Refer to the [crate] docs for more info.
#[derive(Debug, Clone)]
pub struct Flow<T: Flows> {
    state: Arc<FlowState<T>>,
}

impl<T: Flows> Flow<T> {
    /// Returns a tuple of `(flow, [subscribers])`,
    /// where `capacity` is the maximum capacity
    /// of the flow.
    ///
    /// # Panics
    ///
    /// Iff `capacity` is _not_ a power of two
    /// (like `2`, `32`, `256`, and so on).
    pub fn new<const SUB: usize>(capacity: usize) -> (Self, [FlowSubscriber<T>; SUB])
    where
        T: Default,
    {
        assert!(capacity & (capacity - 1) == 0, "flow capacity _must_ be a power of two (like `2`, `4`, `256`, `2048`...), not {capacity}");

        // Allocate the flow buffer.
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(UnsafeCell::new(T::default()));
        }
        let buffer = buffer.into_boxed_slice();

        // Build the flow state.
        let mut flow_state = FlowState {
            buffer,
            next_writable_seq: AtomicU64::new(0),
            next_publishable_seq: AtomicU64::new(0),
            next_receivable_seqs: Vec::with_capacity(SUB),
        };

        // Add subscribers to the state.
        let mut subscriber_seqs = Vec::with_capacity(SUB);
        for _ in 0..SUB {
            subscriber_seqs.push(flow_state.add_subscriber_seq());
        }

        // Finalize flow state and wrap subscriber
        // sequences in the subscriber API.
        let flow_state = Arc::new(flow_state);
        let subscribers: Vec<FlowSubscriber<T>> = subscriber_seqs
            .into_iter()
            .map(|seq| FlowSubscriber {
                flow_state: flow_state.clone(),
                next_receivable_seq: seq,
            })
            .collect();

        (Self { state: flow_state }, subscribers.try_into().unwrap())
    }

    /// Tries to claim the next publishable
    /// sequence in the flow, returning
    /// a [`UnpublishedData`] iff successful.
    pub fn try_next(&mut self) -> Result<UnpublishedData<'_, T>, Error> {
        self.try_next_internal()
    }

    /// Awaits and claims the next publishable sequence
    /// in the flow, returning a [`UnpublishedData`]
    /// iff successful.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> impl Future<Output = Result<UnpublishedData<'_, T>, Error>> {
        PublishNextFuture { flow: self }
    }

    /// Implementation of [`Self::try_next`] that
    /// takes `self` as an immutable reference with
    /// interior mutability.
    #[inline(always)]
    fn try_next_internal(&self) -> Result<UnpublishedData<'_, T>, Error> {
        if let Some(next) = self.state.try_claim_publishable() {
            let next_item = UnpublishedData {
                flow: self,
                sequence: next,
                data: unsafe { self.state.get_mut(next) },
            };
            Ok(next_item)
        } else {
            Err(Error::Full)
        }
    }
}

/// Future returned by [`Flow::next`].
struct PublishNextFuture<'a, T: Flows> {
    flow: &'a Flow<T>,
}

impl<'a, T: Flows> Future for PublishNextFuture<'a, T> {
    type Output = Result<UnpublishedData<'a, T>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.flow.try_next_internal() {
            Ok(next) => Poll::Ready(Ok(next)),
            Err(Error::Full) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Internal state of a [`Flow`].
///
/// This state is placed in a separate data
/// structure from the rest of a [`Flow`]
/// to simplify sharing references to the state
/// between a flow and it's subscribers.
struct FlowState<T: Flows> {
    /// Pre-allocated contiguous buffer of
    /// data entries in the flow.
    ///
    /// This buffer is a ring buffer: When it
    /// is full, writes "wrap" around to the
    /// beginning of the buffer, overwriting
    /// the oldest data.
    ///
    /// Each data entry in the buffer is
    /// wrapped in an [`UnsafeCell`], enabling
    /// concurrent tasks to immutably read
    /// the same data at the same time.
    buffer: Box<[UnsafeCell<T>]>,

    /// The sequence number that will be assigned
    /// to the _next_ data entry written into the flow.
    next_writable_seq: AtomicU64,

    /// The sequence number of the next data entry
    /// that will become readable by the flow's
    /// subscriber(s).
    ///
    /// All data entries with sequences less than
    /// this number are assumed to be readable.
    next_publishable_seq: AtomicU64,

    /// The sequence numbers of the next data entry
    /// that will be read by each of the flow's
    /// subscriber(s).
    ///
    /// All data entries with sequences less than
    /// the _lowest_ of these sequence numbers are
    /// assumed to be overwritable.
    next_receivable_seqs: Vec<Weak<AtomicU64>>,
}

impl<T> FlowState<T>
where
    T: Flows,
{
    /// Adds and returns a new subscriber sequence
    /// number to the flow.
    fn add_subscriber_seq(&mut self) -> Arc<AtomicU64> {
        let next_receivable_seq = Arc::new(AtomicU64::new(0));
        self.next_receivable_seqs
            .push(Arc::downgrade(&next_receivable_seq));
        next_receivable_seq
    }

    /// Tries to claim and return the next
    /// publishable data sequence in the flow.
    ///
    /// Iff `Some(sequence)` is returned, the
    /// sequence _must_ be published via
    /// [`Self::try_publish`], or the flow
    /// will stall from backpressure.
    ///
    /// Iff `None` is returned, the flow is full.
    #[inline(always)]
    fn try_claim_publishable(&self) -> Option<u64> {
        let next_writable = self.next_writable_seq.load(Ordering::SeqCst);

        // Calculate the minimum receivable sequence
        // across all subscribers, defaulting to the
        // current sequence that's publishable.
        let mut min_receivable_seq = self.next_publishable_seq.load(Ordering::SeqCst);
        for next_received_seq in self.next_receivable_seqs.iter() {
            if let Some(seq) = next_received_seq.upgrade() {
                min_receivable_seq = min_receivable_seq.min(seq.load(Ordering::SeqCst));
            }
        }

        // Only claim if there's space.
        if min_receivable_seq + self.buffer.len() as u64 > next_writable
            && self
                .next_writable_seq
                .compare_exchange(
                    next_writable,
                    next_writable + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        {
            return Some(next_writable);
        }

        None
    }

    /// Tries to publish `sequence`, returning
    /// true iff the sequence was published.
    #[inline(always)]
    fn try_publish(&self, sequence: u64) -> bool {
        self.next_publishable_seq
            .compare_exchange_weak(sequence, sequence + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Returns a reference to the data at `sequence`.
    ///
    /// Refer to [`Self::get_mut`] for information
    /// on the safety properties of this function.
    ///
    /// # Panics
    ///
    /// Iff any other thread attempts to acquire a _mutable_
    /// reference to `sequence` at the same time.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    unsafe fn get(&self, sequence: u64) -> &T {
        assert!(self.buffer.len() & (self.buffer.len() - 1) == 0);

        // Convert sequence to an queue index.
        let index = (self.buffer.len() - 1) & sequence as usize;

        // Array access will always be within bounds.
        &*self.buffer.get_unchecked(index).get()
    }

    /// Returns a mutable reference to the data at `sequence`.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it _can_ return
    /// multiple mutable references to the sae data.
    ///
    /// This function _is_ safe to call from any task which
    /// has successfully claimed a sequence number via
    /// [`Self::try_claim_publishable`] and
    /// has not yet published that sequence number
    /// via [`Self::try_publish`]. In this scenario,
    /// the task is guaranteed to be the only one with
    /// read/write access to the data.
    ///
    /// This function's behavior is undefined if the task
    /// (having claimed a sequence number via
    /// [`Self::try_claim_publishable`]) calls this
    /// function _repeatedly_ with the same sequence number.
    ///
    /// # Panics
    ///
    /// Iff the same or different tasks attempt to acquire
    /// more than one _mutable_ reference to `sequence`.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    unsafe fn get_mut(&self, sequence: u64) -> &mut T {
        assert!(self.buffer.len() & (self.buffer.len() - 1) == 0);

        // Convert sequence to an queue index.
        let index = (self.buffer.len() - 1) & sequence as usize;

        // Array access will always be within bounds.
        &mut *self.buffer.get_unchecked(index).get()
    }
}

impl<T> Debug for FlowState<T>
where
    T: Flows,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Flow")
            .field("capacity", &self.buffer.len())
            .field("next_writable_seq", &self.next_writable_seq)
            .field("next_publishable_seq", &self.next_publishable_seq)
            .field("next_receivable_seqs", &self.next_receivable_seqs)
            .finish()
    }
}

/// Subscriber which receives data from a [`Flow`].
pub struct FlowSubscriber<T: Flows> {
    flow_state: Arc<FlowState<T>>,

    /// See [`FlowState::next_receivable_seqs`].
    next_receivable_seq: Arc<AtomicU64>,
}

impl<T: Flows> FlowSubscriber<T> {
    /// Returns a reference to the next data
    /// in the flow, if the flow is active and
    /// any data is available.
    pub fn try_next(&mut self) -> Result<impl Deref<Target = T> + '_, Error> {
        self.try_next_internal()
    }

    /// Awaits and returns a reference to the next
    /// data  in the flow, if the flow is active.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> impl Future<Output = Result<impl Deref<Target = T> + '_, Error>> {
        ReceiveNextFuture { subscriber: self }
    }

    /// Implementation of [`Self::try_next`] that
    /// takes `self` as an immutable reference with
    /// interior mutability.
    #[inline(always)]
    fn try_next_internal(&self) -> Result<PublishedData<'_, T>, Error> {
        if let Some(next) = self.receivable_seqs().next() {
            let data = PublishedData {
                subscription: self,
                sequence: next,
                data: unsafe { self.flow_state.get(next) },
            };

            Ok(data)
        } else {
            Err(Error::Ahead)
        }
    }

    /// Returns the range of data sequence numbers
    /// that are receivable by this subscriber.
    #[inline(always)]
    fn receivable_seqs(&self) -> Range<u64> {
        self.next_receivable_seq.load(Ordering::SeqCst)
            ..self.flow_state.next_publishable_seq.load(Ordering::SeqCst)
    }

    /// Marks all sequences up to (and including)
    /// `sequence` as received by this subscriber.
    #[inline(always)]
    fn receive_up_to(&self, sequence: u64) {
        self.next_receivable_seq
            .fetch_max(sequence + 1, Ordering::SeqCst);
    }
}

/// Future returned by [`FlowSubscriber::next`].
struct ReceiveNextFuture<'a, T: Flows> {
    subscriber: &'a FlowSubscriber<T>,
}

impl<'a, T: Flows> Future for ReceiveNextFuture<'a, T> {
    type Output = Result<PublishedData<'a, T>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.subscriber.try_next_internal() {
            Ok(next) => Poll::Ready(Ok(next)),
            Err(Error::Ahead) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<T> Debug for FlowSubscriber<T>
where
    T: Flows,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OutBarrier")
            .field("flow_state", &self.flow_state)
            .field("next_receivable_seq", &self.next_receivable_seq)
            .finish()
    }
}

// Flow states may be sent between threads and
// safely accessed concurrently.
unsafe impl<T> Send for FlowState<T> where T: Flows {}
unsafe impl<T> Sync for FlowState<T> where T: Flows {}

/// Blanket trait for data in a [`Flow`].
pub trait Flows: Send + Sync + 'static {}
impl<T> Flows for T where T: Send + Sync + 'static {}

/// Reference to mutable, unpublished data in a [`Flow`].
///
/// When this reference is dropped, the data
/// is marked as published into the [`Flow`].
#[derive(Debug)]
pub struct UnpublishedData<'a, T: Flows> {
    flow: &'a Flow<T>,
    sequence: u64,
    data: &'a mut T,
}

impl<T: Flows> UnpublishedData<'_, T> {
    /// Returns the data's sequence number.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Publishes `data` into this sequence.
    pub fn publish(self, data: T) {
        *self.data = data;
        drop(self)
    }
}

impl<T: Flows> Deref for UnpublishedData<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T: Flows> DerefMut for UnpublishedData<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl<T: Flows> Drop for UnpublishedData<'_, T> {
    fn drop(&mut self) {
        while !self.flow.state.try_publish(self.sequence) {
            core::hint::spin_loop();
        }
    }
}

/// Return value of [`FlowSubscriber::try_next`].
///
/// When this value is dropped, the data will
/// be marked as received by its corresponding
/// subscriber.
#[derive(Debug)]
struct PublishedData<'a, T: Flows> {
    subscription: &'a FlowSubscriber<T>,
    sequence: u64,
    data: &'a T,
}

impl<T: Flows> Deref for PublishedData<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T: Flows> Drop for PublishedData<'_, T> {
    fn drop(&mut self) {
        self.subscription.receive_up_to(self.sequence);
    }
}

/// Enumeration of non-retryable errors
/// that may happen while using flows.
#[derive(Debug, Snafu, PartialEq)]
pub enum Error {
    /// Publishing is temporarily impossible:
    /// the flow is full of unreceived data.
    Full,

    /// The flow may or may not contain data, but the
    /// subscriber has already read all data presently
    /// in the flow.
    Ahead,
}

#[cfg(test)]
mod test {
    use super::*;

    /// Tests basic API functionality.
    #[test]
    fn pubs_and_subs() -> Result<(), crate::Error> {
        // Prepare pubsub.
        let (mut publisher, [mut subscriber]) = Flow::new(2);

        // Publish some data.
        let mut data = publisher.try_next().unwrap();
        *data = 42u32;
        assert_eq!(0, data.sequence());
        drop(data);

        // Check barrier sequences.
        assert_eq!(0..1, subscriber.receivable_seqs());

        // Receive some data.
        let data = subscriber.try_next().unwrap();
        assert!(42u32 == *data);
        drop(data);

        // Check barrier sequences.
        assert_eq!(1..1, subscriber.receivable_seqs());

        Ok(())
    }
}
