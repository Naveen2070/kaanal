//! The embedded demo project (`kaanal demo`, `demo --eject`): every file the template is made of.

// The tests include this file with `#[path]`, so keep it free of other code; the command is in
// `demo.rs`.
/// (path relative to the project root, contents).
pub const FILES: &[(&str, &str)] = &[
    ("kaanal.yaml", include_str!("../demo/kaanal.yaml")),
    ("openapi.yaml", include_str!("../demo/openapi.yaml")),
    (
        "scenarios/slow-db.yaml",
        include_str!("../demo/scenarios/slow-db.yaml"),
    ),
    (
        "scenarios/cache-down.yaml",
        include_str!("../demo/scenarios/cache-down.yaml"),
    ),
    (
        "scenarios/auth-errors.yaml",
        include_str!("../demo/scenarios/auth-errors.yaml"),
    ),
    (
        "scenarios/black-friday.yaml",
        include_str!("../demo/scenarios/black-friday.yaml"),
    ),
    (
        "scenarios/flaky-network.yaml",
        include_str!("../demo/scenarios/flaky-network.yaml"),
    ),
    (
        "scenarios/gateway-down.yaml",
        include_str!("../demo/scenarios/gateway-down.yaml"),
    ),
    (
        "scenarios/db-timeout.yaml",
        include_str!("../demo/scenarios/db-timeout.yaml"),
    ),
    (
        "scenarios/cart-write-errors.yaml",
        include_str!("../demo/scenarios/cart-write-errors.yaml"),
    ),
    (
        "scenarios/brownout.yaml",
        include_str!("../demo/scenarios/brownout.yaml"),
    ),
    (
        "scenarios/meltdown.yaml",
        include_str!("../demo/scenarios/meltdown.yaml"),
    ),
    (
        "scenarios/payment-slow.yaml",
        include_str!("../demo/scenarios/payment-slow.yaml"),
    ),
    (
        "scenarios/payment-flaky.yaml",
        include_str!("../demo/scenarios/payment-flaky.yaml"),
    ),
    (
        "scenarios/payment-down.yaml",
        include_str!("../demo/scenarios/payment-down.yaml"),
    ),
    (
        "scenarios/payment-timeout.yaml",
        include_str!("../demo/scenarios/payment-timeout.yaml"),
    ),
    (
        "scenarios/payment-declined.yaml",
        include_str!("../demo/scenarios/payment-declined.yaml"),
    ),
    ("site/index.html", include_str!("../demo/site/index.html")),
    ("site/app.js", include_str!("../demo/site/app.js")),
    ("site/style.css", include_str!("../demo/site/style.css")),
    (
        "site/img/product.svg",
        include_str!("../demo/site/img/product.svg"),
    ),
];
