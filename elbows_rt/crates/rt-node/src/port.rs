//! Publishers and subscribers over whichever transport the node uses.
//!
//! Each is an enum over the two backends, so calling code sees one type. The `match` costs one
//! branch per call. A local node's ports are always the iceoryx2 variant, which never allocates,
//! encodes or blocks: that, not the enum, is what keeps the RT thread real-time safe. The Zenoh
//! variant allocates (encoding) and may lock, so it belongs off the RT thread.

use rt_core::Message;
use rt_ipc::{IpcPublisher, IpcSubscriber};
use rt_net::{NetPublisher, NetSubscriber};

use crate::NodeError;

pub enum Publisher<T: Message> {
    Ipc(IpcPublisher<T>),
    Net(NetPublisher<T>),
}

impl<T: Message> Publisher<T> {
    /// Send `value` to every subscriber. Never blocks: a full subscriber buffer drops its oldest.
    pub fn publish(&self, value: &T) -> Result<(), NodeError> {
        match self {
            Self::Ipc(p) => p.publish(value)?,
            Self::Net(p) => p.publish(value)?,
        }
        Ok(())
    }
}

pub enum Subscriber<T: Message> {
    Ipc(IpcSubscriber<T>),
    Net(NetSubscriber<T>),
}

impl<T: Message> Subscriber<T> {
    /// The newest unread message, if any; older unread ones are discarded. It came from another
    /// process: validate it before use.
    pub fn latest(&self) -> Result<Option<T>, NodeError> {
        match self {
            Self::Ipc(s) => Ok(s.latest()?),
            Self::Net(s) => Ok(s.latest()),
        }
    }

    /// Every unread message, oldest first. Returns how many there were. Over the network, messages
    /// that don't decode as `T` (another type or layout version) are skipped.
    pub fn drain(&self, mut f: impl FnMut(T)) -> Result<usize, NodeError> {
        match self {
            Self::Ipc(s) => Ok(s.drain(f)?),
            Self::Net(s) => {
                let mut n = 0;
                for msg in s.drain().flatten() {
                    f(msg);
                    n += 1;
                }
                Ok(n)
            }
        }
    }
}
