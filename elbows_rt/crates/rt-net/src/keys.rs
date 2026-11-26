//! Where everything lives in Zenoh's key space.
//!
//! ```text
//! elbows/<robot>/topic/<name>        topics, encoded by their Codec
//! elbows/<robot>/srv/<name>          services: JSON request → JSON response. The serving node also
//!                                    holds a liveliness token here, which is how one owner is enforced
//! elbows/<robot>/node/<name>/<id>    liveliness token of each node on the network
//! ```

use rt_core::services::Service;
use rt_core::topics::Topic;

const ROOT: &str = "elbows";

pub fn topic_key<T>(robot: &str, topic: &Topic<T>) -> String {
    format!("{ROOT}/{robot}/topic/{}", topic.name)
}

pub fn service_key<Req, Resp>(robot: &str, service: &Service<Req, Resp>) -> String {
    format!("{ROOT}/{robot}/srv/{}", service.name)
}

pub fn node_key(robot: &str, name: &str, id: &str) -> String {
    format!("{ROOT}/{robot}/node/{name}/{id}")
}

pub fn nodes_wildcard(robot: &str) -> String {
    format!("{ROOT}/{robot}/node/**")
}

/// `<name>/<id>` from a node key, for display.
pub fn node_label(key: &str) -> &str {
    key.split("/node/").nth(1).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(
            service_key("so101", &rt_core::services::ENABLE),
            "elbows/so101/srv/enable"
        );
        assert_eq!(
            topic_key("so101", &rt_core::topics::PLAN),
            "elbows/so101/topic/plan"
        );
        assert_eq!(node_label("elbows/so101/node/policy/abc"), "policy/abc");
    }
}
