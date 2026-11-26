//! Publishers and subscribers for any declared topic, over Zenoh. Encoding is the codec's job
//! (`Raw` by default).
//!
//! Nothing here can block Zenoh: Zenoh delivers into each subscriber's buffer on its own receive
//! thread, and a full buffer drops its oldest message (as iceoryx2 does) rather than making that
//! thread wait, which would stall every subscriber on the session. The buffer is sized by the
//! topic's pattern: 1 for `Latest`, `depth` for `Queue`.

use std::marker::PhantomData;

use rt_core::topics::{Pattern, Topic};
use zenoh::handlers::{RingChannel, RingChannelHandler};
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
    subscriber: zenoh::pubsub::Subscriber<RingChannelHandler<Sample>>,
    _types: PhantomData<fn() -> (T, C)>,
}

impl<T, C: Codec<T>> NetSubscriber<T, C> {
    pub fn new(session: &Session, robot: &str, topic: &Topic<T>) -> Result<Self, NetError> {
        let capacity = match topic.pattern {
            Pattern::Latest => 1,
            Pattern::Queue { depth } => depth,
        };
        let subscriber = session
            .declare_subscriber(topic_key(robot, topic))
            .with(RingChannel::new(capacity))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            subscriber,
            _types: PhantomData,
        })
    }

    /// Every message still buffered, oldest first; undecodable ones as errors.
    pub fn drain(&self) -> impl Iterator<Item = Result<T, NetError>> + '_ {
        std::iter::from_fn(|| self.subscriber.try_recv().ok().flatten())
            .map(|s| C::decode(&s.payload().to_bytes()))
    }

    /// The newest valid message received since the last call, if any.
    pub fn latest(&self) -> Option<T> {
        self.drain().filter_map(Result::ok).last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{NetConfig, open};
    use bytemuck::Zeroable;
    use rt_core::msgs::ArmState;
    use rt_core::topics::STATE;
    use std::time::Duration;

    const QUEUE: Topic<ArmState> = Topic::queue("test_queue", 4);

    fn state(cycle: u64) -> ArmState {
        ArmState {
            cycle,
            ..ArmState::zeroed()
        }
    }

    /// A subscriber that isn't read fills up and drops its oldest, without stalling the session:
    /// later messages still arrive, on every topic.
    #[test]
    fn unread_subscribers_drop_oldest_and_never_stall() {
        let endpoint = format!("tcp/127.0.0.1:{}", 19_000 + std::process::id() % 1000);
        let a = open(&NetConfig::server(std::slice::from_ref(&endpoint))).unwrap();
        let b = open(&NetConfig::client(&endpoint)).unwrap();
        let (latest_out, queue_out) = (
            NetPublisher::<ArmState>::new(&a, "t", &STATE).unwrap(),
            NetPublisher::<ArmState>::new(&a, "t", &QUEUE).unwrap(),
        );
        let latest = NetSubscriber::<ArmState>::new(&b, "t", &STATE).unwrap();
        let queue = NetSubscriber::<ArmState>::new(&b, "t", &QUEUE).unwrap();
        let until = |cycle: u64| {
            (0..200).any(|_| {
                latest_out.publish(&state(cycle)).unwrap();
                queue_out.publish(&state(cycle)).unwrap();
                std::thread::sleep(Duration::from_millis(10));
                latest.latest().is_some_and(|s| s.cycle == cycle)
            })
        };
        assert!(until(0), "connected");
        queue.drain().for_each(drop);

        // Far more than Zenoh's default 256-message buffer, none of it read.
        for cycle in 1..=2_000 {
            latest_out.publish(&state(cycle)).unwrap();
            queue_out.publish(&state(cycle)).unwrap();
        }
        assert!(until(2_001), "the session still delivers");
        let kept: Vec<u64> = queue.drain().map(|s| s.unwrap().cycle).collect();
        assert!(kept.len() <= 4, "a queue keeps its depth: {kept:?}");
        assert_eq!(kept.last(), Some(&2_001));
    }
}
