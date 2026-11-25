//! Publishers and subscribers for any declared topic, over Zenoh. Encoding is the codec's job
//! (`Raw` by default).

use std::marker::PhantomData;

use rt_core::topics::Topic;
use zenoh::handlers::FifoChannelHandler;
use zenoh::sample::Sample;
use zenoh::{Session, Wait};

use crate::codec::{Codec, Raw};
use crate::keys::topic_key;
use crate::{NetError, zerr};

pub struct NetPublisher<T, C = Raw> {
    publisher: zenoh::pubsub::Publisher<'static>,
    _types: PhantomData<fn(T, C)>,
}

impl<T, C: Codec<T>> NetPublisher<T, C> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let publisher = session
            .declare_publisher(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            publisher,
            _types: PhantomData,
        })
    }

    pub fn publish(&self, value: &T) -> Result<(), NetError> {
        self.publisher.put(C::encode(value)).wait().map_err(zerr)
    }
}

pub struct NetSubscriber<T, C = Raw> {
    subscriber: zenoh::pubsub::Subscriber<FifoChannelHandler<Sample>>,
    _types: PhantomData<fn() -> (T, C)>,
}

impl<T, C: Codec<T>> NetSubscriber<T, C> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let subscriber = session
            .declare_subscriber(topic_key(robot, topic))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            subscriber,
            _types: PhantomData,
        })
    }

    /// Every message received since the last call, oldest first; undecodable ones as errors.
    pub fn drain(&self) -> impl Iterator<Item = Result<T, NetError>> + '_ {
        std::iter::from_fn(|| self.subscriber.try_recv().ok().flatten())
            .map(|s| C::decode(&s.payload().to_bytes()))
    }

    /// The newest valid message received since the last call, if any.
    pub fn latest(&self) -> Option<T> {
        self.drain().filter_map(Result::ok).last()
    }
}
