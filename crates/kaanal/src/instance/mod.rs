//! The running instance as the CLI sees it: the lock and `run.json`, the signing key, which instance
//! a command talks to, and the HTTP client for its control API.

pub(crate) mod keys;
pub(crate) mod lifecycle;
pub(crate) mod remote;
pub(crate) mod state_dir;
pub(crate) mod target;
