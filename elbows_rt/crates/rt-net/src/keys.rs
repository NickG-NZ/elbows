//! Where everything lives in Zenoh's key space.
//!
//! ```text
//! elbows/<robot>/topic/<name>           topics, encoded by their Codec
//! elbows/<robot>/srv/<name>             services: JSON request → JSON response
//! elbows/<robot>/session/<role>/<id>    liveliness tokens of connected clients
//! ```

use rt_core::services::Service;
use rt_core::topics::Topic;

const ROOT: &str = "elbows";

pub fn topic_key<T>(robot: &str, topic: &Topic<T>) -> String {
    format!("{ROOT}/{robot}/topic/{}", topic.name)
}

pub fn service_key<Req, Resp>(robot: &str, service: &Service<Req, Resp>) -> String {
    format!("{}{}", services_prefix(robot), service.name)
}

/// Everything before a service's name; a server listens on this plus `*`.
pub fn services_prefix(robot: &str) -> String {
    format!("{ROOT}/{robot}/srv/")
}

pub fn session_key(robot: &str, role: &str, id: &str) -> String {
    format!("{ROOT}/{robot}/session/{role}/{id}")
}

pub fn sessions_wildcard(robot: &str) -> String {
    format!("{ROOT}/{robot}/session/**")
}

/// `<role>/<id>` from a session key, for display.
pub fn session_name(key: &str) -> &str {
    key.split("/session/").nth(1).unwrap_or(key)
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
        assert_eq!(
            session_name("elbows/so101/session/policy/abc"),
            "policy/abc"
        );
    }
}
