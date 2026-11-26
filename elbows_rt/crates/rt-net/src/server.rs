//! The provider side of services (`rt_core::services`): a [`Server`] runs the handlers registered
//! with it. Callers use [`call`](crate::call). Requests and responses are JSON.
//!
//! Each service has **one owner**: registering a service claims its key (a queryable plus a
//! liveliness token on it), and fails if another live node already holds it. Callers name only the
//! service, so a second server for the same name would make calls ambiguous. The claim lapses when
//! its owner exits or crashes, so a restarted owner can claim it again.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

use rt_core::services::Service;
use serde::Serialize;
use serde::de::DeserializeOwned;
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};
use zenoh::{Session, Wait};

use crate::codec::{Codec, Json};
use crate::keys::service_key;
use crate::{NetError, zerr};

// Requests waiting to be served. Beyond this, new ones are dropped (the caller gets no reply) rather
/// than queueing without bound while the owner isn't polling.
pub const PENDING: usize = 64;

/// How long registering waits to hear about an existing owner.
const CLAIM_TIMEOUT: Duration = Duration::from_millis(500);

type Handler<Ctx> = Box<dyn FnMut(&mut Ctx, &[u8]) -> Result<Vec<u8>, String> + Send>;

struct Claim {
    _queryable: Queryable<()>,
    _token: LivelinessToken,
}

/// Serves the services registered with [`handle`](Server::handle). Handlers get `&mut Ctx`, the
/// state they act on, which the caller passes to [`poll`](Server::poll); they run on the polling
/// thread, nowhere else.
pub struct Server<Ctx> {
    session: Session,
    robot: String,
    requests: Receiver<(&'static str, Query)>,
    sender: SyncSender<(&'static str, Query)>,
    handlers: HashMap<&'static str, Handler<Ctx>>,
    claims: Vec<Claim>,
}

impl<Ctx> Server<Ctx> {
    pub fn new(session: &Session, robot: &str) -> Self {
        let (sender, requests) = sync_channel(PENDING);
        Self {
            session: session.clone(),
            robot: robot.to_string(),
            requests,
            sender,
            handlers: HashMap::new(),
            claims: Vec::new(),
        }
    }

    /// Serve `service` with `handler`. Fails with [`NetError::Owned`] if another node serves it.
    pub fn handle<Req, Resp>(
        &mut self,
        service: &Service<Req, Resp>,
        mut handler: impl FnMut(&mut Ctx, Req) -> Resp + Send + 'static,
    ) -> Result<&mut Self, NetError>
    where
        Req: Serialize + DeserializeOwned + 'static,
        Resp: Serialize + DeserializeOwned + 'static,
    {
        let key = service_key(&self.robot, service);
        if self.handlers.contains_key(service.name) || self.owned_elsewhere(&key)? {
            return Err(NetError::Owned { key });
        }
        let name = service.name;
        let sender = self.sender.clone();
        let queryable = self
            .session
            .declare_queryable(&key)
            .complete(true)
            .callback(move |query| {
                // Never blocks Zenoh: if full (or the server is gone), the query is dropped unanswered.
                let _ = sender.try_send((name, query));
            })
            .wait()
            .map_err(zerr)?;
        let token = self
            .session
            .liveliness()
            .declare_token(&key)
            .wait()
            .map_err(zerr)?;
        self.claims.push(Claim {
            _queryable: queryable,
            _token: token,
        });
        self.handlers.insert(
            name,
            Box::new(move |ctx, bytes| {
                let req =
                    <Json as Codec<Req>>::decode(bytes).map_err(|e| format!("bad request: {e}"))?;
                Ok(<Json as Codec<Resp>>::encode(&handler(ctx, req)))
            }),
        );
        Ok(self)
    }

    fn owned_elsewhere(&self, key: &str) -> Result<bool, NetError> {
        let replies = self
            .session
            .liveliness()
            .get(key)
            .timeout(CLAIM_TIMEOUT)
            .wait()
            .map_err(zerr)?;
        Ok(std::iter::from_fn(|| replies.recv().ok()).any(|r| r.result().is_ok()))
    }

    /// Wait up to `timeout` for a request, then serve it and any others already waiting.
    /// Returns how many were served.
    pub fn poll(&mut self, ctx: &mut Ctx, timeout: Duration) -> usize {
        let Ok(first) = self.requests.recv_timeout(timeout) else {
            return 0;
        };
        let mut served = 0;
        let mut next = Some(first);
        while let Some((name, query)) = next {
            self.serve(ctx, name, &query);
            served += 1;
            next = self.requests.try_recv().ok();
        }
        served
    }

    fn serve(&mut self, ctx: &mut Ctx, name: &'static str, query: &Query) {
        let payload = query
            .payload()
            .map(|p| p.to_bytes().into_owned())
            .unwrap_or_default();
        let handler = self.handlers.get_mut(name).expect("claimed with a handler");
        let sent = match handler(ctx, &payload) {
            Ok(bytes) => query.reply(query.key_expr().clone(), bytes).wait(),
            Err(msg) => query.reply_err(msg).wait(),
        };
        if let Err(e) = sent {
            eprintln!("rt-net: replying to {}: {e}", query.key_expr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call;
    use crate::session::{NetConfig, open};
    use rt_core::services::{ENABLE, HOLD, Reply};

    fn endpoint(offset: u32) -> String {
        format!(
            "tcp/127.0.0.1:{}",
            17_000 + (std::process::id() % 400) * 2 + offset
        )
    }

    /// A real round trip through Zenoh on a private localhost port.
    #[test]
    fn server_dispatches_by_service() {
        let endpoint = endpoint(0);
        let server_session = open(&NetConfig::server(std::slice::from_ref(&endpoint))).unwrap();
        let client = open(&NetConfig::client(&endpoint)).unwrap();
        let mut server = Server::<u32>::new(&server_session, "t");
        server
            .handle(&ENABLE, |calls: &mut u32, ()| {
                *calls += 1;
                Reply {
                    ok: true,
                    message: format!("call {calls}"),
                    ..Default::default()
                }
            })
            .unwrap();
        let serving = std::thread::spawn(move || {
            let mut calls = 0;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while calls < 2 && std::time::Instant::now() < deadline {
                server.poll(&mut calls, Duration::from_millis(10));
            }
            drop(server_session);
        });
        // the client may need a moment to connect
        let first = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(20));
                call(&client, "t", &ENABLE, &()).ok()
            })
            .unwrap();
        assert_eq!(first.message, "call 1");
        assert!(
            matches!(
                call(&client, "t", &HOLD, &()),
                Err(NetError::NoReply { .. })
            ),
            "nobody serves hold"
        );
        assert_eq!(call(&client, "t", &ENABLE, &()).unwrap().message, "call 2");
        serving.join().unwrap();
    }

    #[test]
    fn a_service_has_one_owner() {
        let endpoint = endpoint(1);
        let a = open(&NetConfig::server(std::slice::from_ref(&endpoint))).unwrap();
        let b = open(&NetConfig::client(&endpoint)).unwrap();
        let reply = |_: &mut (), ()| Reply::default();
        let mut first = Server::new(&a, "t");
        first.handle(&ENABLE, reply).unwrap();
        assert!(matches!(
            first.handle(&ENABLE, reply),
            Err(NetError::Owned { .. })
        ));
        // another node, once connected, sees the claim
        let mut second = Server::new(&b, "t");
        let refused = (0..50).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            matches!(second.handle(&ENABLE, reply), Err(NetError::Owned { .. }))
        });
        assert!(refused);
        // a different service is free
        second.handle(&HOLD, reply).unwrap();
        // the claim lapses with its owner
        drop(first);
        let claimed = (0..50).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            second.handle(&ENABLE, reply).is_ok()
        });
        assert!(claimed);
    }
}
