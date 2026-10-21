//! The `--json` switch: one global bit, set once in `main`, read where a command prints.

use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

/// **Private.** Whether commands print JSON instead of text.
static JSON: AtomicBool = AtomicBool::new(false);

/// **Private.** Turns JSON output on or off.
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

/// **Private.** Whether JSON output is on.
pub fn json_on() -> bool {
    JSON.load(Ordering::Relaxed)
}

/// **Private.** `KAANAL_JSON` counts as the flag when it is `1` or `true`; empty or anything
/// else is off.
pub fn env_on() -> bool {
    std::env::var("KAANAL_JSON").is_ok_and(|v| v == "1" || v == "true")
}

/// **Private.** Prints `v` as one compact line in JSON mode (nothing for `Null`), else runs `human`.
pub fn emit(v: &Value, human: impl FnOnce()) {
    if !json_on() {
        human();
    } else if !v.is_null() {
        crate::out_line(&v.to_string());
    }
}
