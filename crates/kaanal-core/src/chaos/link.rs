//! The rules of chaos on a link (an edge): which fields it takes and which values it allows.

use super::{ChaosLayer, ErrorKind};

impl ChaosLayer {
    /// The first link-only field this layer sets (`jitter`, `bandwidth_cap`), if any: a node,
    /// endpoint or replica key may not carry one.
    #[must_use]
    pub fn link_only_field(&self) -> Option<&'static str> {
        if self.jitter.is_some() {
            Some("jitter")
        } else if self.bandwidth_cap.is_some() {
            Some("bandwidth_cap")
        } else {
            None
        }
    }

    /// The extra rules of a layer on an edge whose own `bandwidth` is `bandwidth`: a link has no
    /// status (only `reset` or `hang` kinds, no `error_status`), and a `bandwidth_cap` only lowers
    /// an existing bandwidth. Run [`ChaosLayer::check`] too.
    ///
    /// # Errors
    /// The first offending field and a message about it.
    pub fn check_link(&self, bandwidth: Option<u64>) -> Result<(), (&'static str, String)> {
        if self.error_status.is_some() {
            return Err((
                "error_status",
                "a link has no status; drop error_status".into(),
            ));
        }
        if matches!(
            self.error_kind,
            Some(ErrorKind::Status | ErrorKind::Malformed)
        ) {
            return Err((
                "error_kind",
                "a link fails with reset or hang, not status or malformed".into(),
            ));
        }
        if let Some(cap) = self.bandwidth_cap {
            match bandwidth {
                None => {
                    return Err((
                        "bandwidth_cap",
                        "the edge has no bandwidth to cap; set bandwidth on the edge first".into(),
                    ));
                }
                Some(b) if cap > b => {
                    return Err((
                        "bandwidth_cap",
                        format!(
                            "bandwidth_cap {cap} is above the edge's bandwidth {b}; a cap only lowers it"
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}
