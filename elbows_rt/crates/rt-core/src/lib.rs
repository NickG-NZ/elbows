//! Declarations shared by every process (messages, topics, services) and building blocks for the RT
//! loop. Nothing here allocates or blocks except where documented.

use std::fmt::Debug;

use bytemuck::Pod;
use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;

pub mod msgs;
pub mod rt;
pub mod services;
pub mod stats;
pub mod time;
pub mod topics;
pub mod trigger;

/// A type that can be sent on a topic: plain data, valid for any bytes a peer writes (`Pod`), with
/// a fixed layout and a stable, versioned name (`ZeroCopySend`'s `type_name`, e.g. `elbows.Plan/1`).
/// The same type goes through shared memory (iceoryx2) and over the network (Zenoh).
pub trait Message: Pod + ZeroCopySend + Debug + Send + Sync + 'static {}
impl<T: Pod + ZeroCopySend + Debug + Send + Sync + 'static> Message for T {}

/// The stable name a message is identified by across processes and machines.
pub fn type_name<T: Message>() -> &'static str {
    // SAFETY: only reads the type's declared name.
    unsafe { <T as ZeroCopySend>::type_name() }
}
