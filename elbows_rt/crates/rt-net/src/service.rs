//! Request/response for any declared service: a [`Server`] serves the handlers registered with it,
//! clients use [`call`]. Requests and responses are JSON.

use std::collections::HashMap;
use std::time::Duration;

use rt_core::services::Service;
use serde::Serialize;
use serde::de::DeserializeOwned;
use zenoh::handlers::FifoChannelHandler;
use zenoh::query::{Query, Queryable};
use zenoh::{Session, Wait};

use crate::codec::{Codec, Json};
use crate::keys::{service_key, services_prefix};
use crate::{NetError, zerr};

/// How long a client waits for a service reply.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);

type Handler<Ctx> = Box<dyn FnMut(&mut Ctx, &[u8]) -> Result<Vec<u8>, String> + Send>;

/// Serves the services registered with [`handle`](Server::handle). Handlers get `&mut Ctx`, the
/// state they act on, which the caller passes to [`poll`](Server::poll).
pub struct Server<Ctx> {
    queryable: Queryable<FifoChannelHandler<Query>>,
    prefix: String,
    handlers: HashMap<&'static str, Handler<Ctx>>,
}

impl<Ctx> Server<Ctx> {
    /// Serve `elbows/<robot>/srv/*`.
    pub fn new(session: &Session, robot: &str) -> Result<Self, NetError> {
        let prefix = services_prefix(robot);
        let queryable = session
            .declare_queryable(format!("{prefix}*"))
            .wait()
            .map_err(zerr)?;
        Ok(Self {
            queryable,
            prefix,
            handlers: HashMap::new(),
        })
    }

    pub fn handle<Req, Resp>(
        &mut self,
        service: &Service<Req, Resp>,
        mut handler: impl FnMut(&mut Ctx, Req) -> Resp + Send + 'static,
    ) -> &mut Self
    where
        Req: Serialize + DeserializeOwned + 'static,
        Resp: Serialize + DeserializeOwned + 'static,
    {
        let erased: Handler<Ctx> = Box::new(move |ctx, bytes| {
            let req =
                <Json as Codec<Req>>::decode(bytes).map_err(|e| format!("bad request: {e}"))?;
            Ok(<Json as Codec<Resp>>::encode(&handler(ctx, req)))
        });
        self.handlers.insert(service.name, erased);
        self
    }

    /// Wait up to `timeout` for a request, then serve it and any others already waiting.
    /// Returns how many were served.
    pub fn poll(&mut self, ctx: &mut Ctx, timeout: Duration) -> usize {
        let Ok(Some(first)) = self.queryable.recv_timeout(timeout) else {
            return 0;
        };
        let mut served = 0;
        let mut next = Some(first);
        while let Some(query) = next {
            self.serve(ctx, &query);
            served += 1;
            next = self.queryable.try_recv().ok().flatten();
        }
        served
    }

    fn serve(&mut self, ctx: &mut Ctx, query: &Query) {
        let key = query.key_expr().as_str();
        let name = key.strip_prefix(self.prefix.as_str()).unwrap_or(key);
        let payload = query
            .payload()
            .map(|p| p.to_bytes().into_owned())
            .unwrap_or_default();
        let result = match self.handlers.get_mut(name) {
            Some(handler) => handler(ctx, &payload),
            None => Err(format!("unknown service {name:?}")),
        };
        let sent = match result {
            Ok(bytes) => query.reply(query.key_expr().clone(), bytes).wait(),
            Err(msg) => query.reply_err(msg).wait(),
        };
        if let Err(e) = sent {
            eprintln!("rt-net: replying to {key}: {e}");
        }
    }
}

/// Call a service on the robotd for `robot` and wait for its response.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{NetConfig, open};
    use rt_core::services::{ENABLE, HOLD, Reply};

    /// A real round trip through Zenoh on a private localhost port.
    #[test]
    fn server_dispatches_by_service() {
        let endpoint = format!("tcp/127.0.0.1:{}", 17_000 + std::process::id() % 1000);
        let server_session = open(&NetConfig {
            listen: vec![endpoint.clone()],
            connect: Vec::new(),
        })
        .unwrap();
        let client = open(&NetConfig::client(&endpoint)).unwrap();
        let mut server = Server::<u32>::new(&server_session, "t").unwrap();
        server.handle(&ENABLE, |calls: &mut u32, ()| {
            *calls += 1;
            Reply {
                ok: true,
                message: format!("call {calls}"),
                ..Default::default()
            }
        });
        let serving = std::thread::spawn(move || {
            let mut calls = 0;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while calls < 2 && std::time::Instant::now() < deadline {
                server.poll(&mut calls, Duration::from_millis(10));
            }
            drop(server_session);
        });
        // the client may need a moment to connect
        let first = (0..50)
            .find_map(|_| call(&client, "t", &ENABLE, &()).ok())
            .unwrap();
        assert_eq!(first.message, "call 1");
        assert!(
            matches!(call(&client, "t", &HOLD, &()), Err(NetError::Remote { .. })),
            "unregistered"
        );
        assert_eq!(call(&client, "t", &ENABLE, &()).unwrap().message, "call 2");
        serving.join().unwrap();
    }
}
