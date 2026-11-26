//! The caller side of services: [`call`] one by its declaration and wait for the response.

use std::time::Duration;

use rt_core::services::Service;
use serde::Serialize;
use serde::de::DeserializeOwned;
use zenoh::{Session, Wait};

use crate::codec::{Codec, Json};
use crate::keys::service_key;
use crate::{NetError, zerr};

/// How long a caller waits for a reply.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Call `service` on whichever node serves it for `robot`, and wait for its response.
pub fn call<Req, Resp>(
    session: &Session,
    robot: &str,
    service: &Service<Req, Resp>,
    request: &Req,
) -> Result<Resp, NetError>
where
    Req: Serialize + DeserializeOwned,
    Resp: Serialize + DeserializeOwned,
{
    let key = service_key(robot, service);
    let replies = session
        .get(&key)
        .payload(<Json as Codec<Req>>::encode(request))
        .timeout(CALL_TIMEOUT)
        .wait()
        .map_err(zerr)?;
    let reply = replies
        .recv()
        .map_err(|_| NetError::NoReply { key: key.clone() })?;
    match reply.into_result() {
        Ok(sample) => <Json as Codec<Resp>>::decode(&sample.payload().to_bytes()),
        Err(e) => Err(NetError::Remote {
            key,
            msg: e.payload().try_to_string().unwrap_or_default().into_owned(),
        }),
    }
}
