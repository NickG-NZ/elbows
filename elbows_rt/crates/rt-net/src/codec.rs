//! How messages become bytes on the network. Publishers, subscribers and bridges are generic over
//! the codec, so a topic can switch encoding (e.g. to protobuf, for clients in other languages)
//! without touching anything else.

use rt_core::{Message, type_name};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::NetError;

pub trait Codec<T> {
    fn encode(value: &T) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Result<T, NetError>;
}

/// The message's own `#[repr(C)]` bytes, prefixed by its versioned type name (`elbows.Plan/1\0`),
/// so a peer built from a different layout is refused rather than misread. Little-endian. The
/// default for topics.
pub struct Raw;

impl<T: Message> Codec<T> for Raw {
    fn encode(value: &T) -> Vec<u8> {
        let name = type_name::<T>().as_bytes();
        let mut out = Vec::with_capacity(name.len() + 1 + size_of::<T>());
        out.extend_from_slice(name);
        out.push(0);
        out.extend_from_slice(bytemuck::bytes_of(value));
        out
    }

    fn decode(bytes: &[u8]) -> Result<T, NetError> {
        let split = bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or(NetError::Decode("no type name".into()))?;
        let (name, body) = (&bytes[..split], &bytes[split + 1..]);
        if name != type_name::<T>().as_bytes() || body.len() != size_of::<T>() {
            return Err(NetError::Decode(format!(
                "got {:?} ({} bytes), expected {:?} ({} bytes)",
                String::from_utf8_lossy(name),
                body.len(),
                type_name::<T>(),
                size_of::<T>()
            )));
        }
        Ok(bytemuck::pod_read_unaligned(body))
    }
}

/// JSON, for services (and any topic that should be readable by anything).
pub struct Json;

impl<T: Serialize + DeserializeOwned> Codec<T> for Json {
    fn encode(value: &T) -> Vec<u8> {
        serde_json::to_vec(value).expect("serializable")
    }

    fn decode(bytes: &[u8]) -> Result<T, NetError> {
        // An empty payload is `null`, i.e. `()` for requests without arguments.
        let bytes = if bytes.is_empty() {
            b"null".as_slice()
        } else {
            bytes
        };
        serde_json::from_slice(bytes).map_err(|e| NetError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceoryx2_bb_derive_macros::ZeroCopySend;
    use iceoryx2_bb_elementary_traits::zero_copy_send::ZeroCopySend;
    use rt_core::services::Reply;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Pair/1")]
    struct Pair {
        a: u64,
        b: f32,
        c: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable, ZeroCopySend)]
    #[type_name("elbows.test.Pair/2")]
    struct PairV2 {
        a: u64,
        c: u32,
        b: f32,
    }

    #[test]
    fn raw_round_trips_and_refuses_other_layouts() {
        let v = Pair { a: 7, b: 1.5, c: 3 };
        let bytes = <Raw as Codec<Pair>>::encode(&v);
        assert_eq!(<Raw as Codec<Pair>>::decode(&bytes).unwrap(), v);
        // same size, reordered fields, bumped version: refused, not misread
        assert!(<Raw as Codec<PairV2>>::decode(&bytes).is_err());
        assert!(<Raw as Codec<Pair>>::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(<Raw as Codec<Pair>>::decode(b"").is_err());
    }

    #[test]
    fn json_handles_empty_requests() {
        assert_eq!(<Json as Codec<()>>::decode(b"").unwrap(), ());
        let reply = Reply {
            ok: true,
            message: "enabled".into(),
            ..Default::default()
        };
        let bytes = <Json as Codec<Reply>>::encode(&reply);
        assert!(bytes.starts_with(br#"{"ok":true,"message":"enabled""#));
        assert_eq!(<Json as Codec<Reply>>::decode(&bytes).unwrap(), reply);
    }
}
