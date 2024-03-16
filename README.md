# worker-ratelimit [![crates.io][crates-img]][crates] [![docs.rs][docs-img]][docs]

[crates-img]:   https://img.shields.io/crates/v/worker-ratelimit.svg
[crates]:       https://crates.io/crates/worker-ratelimit
[docs-img]:     https://docs.rs/worker-ratelimit/badge.svg
[docs]:         https://docs.rs/worker-ratelimit

This is a general purpose rate limiting library for Cloudflare Workers. It builds on top of Cloudflare Workers KV for storage.

This library is meant to compile to WebAssembly and execute on Cloudflare's serverless plattform. Please note, this is not an official Cloudflare project and implements rate-limiting non-atomically, on best-effort basis, meaning it may fail to count some actions happening in quick succession. Users may be able to go slightly above the configured limits, but once they are reached the limit is effective.

This library is meant to work with features available on free-tier. For more serious use definitely also consider the "Security > WAF > Rate limiting rules" settings.

## Usage

Configure a `RateLimiter` struct containing your rules.

```rust
use worker_ratelimit::RateLimiter;

pub fn setup_ratelimiter() -> RateLimiter {
    // Use the `ratelimit/<key>` namespace within the KV store
    let mut limits = RateLimiter::new("ratelimit");
    // Allow no more than 2 actions within 5 seconds
    limits.add_limit(Duration::from_secs(5), 2);
    // Allow no more than 10 actions within 1 minute
    limits.add_limit(Duration::from_secs(60), 10);
    // Allow no more than 50 actions within 1 hour
    limits.add_limit(Duration::from_secs(3600), 50);
    limits
}
```

Iteract with your KV-backed ratelimits like this:

```rust
let ratelimits = setup_ratelimiter();

// Get the request ip address
let ip_addr = req.headers().get("x-real-ip")?.unwrap_or_else(String::new);

// Check the rate-limit thresholds (also providing the current time)
let Permit::Allow(ticket) = ratelimits.check_kv(&kv, &ip_addr, &Date::now()).await? else {
    return Response::error("Rate limit exceeded", 429);
};

/* Perform the action */
do_something().await?;

// Increase the ratelimit counter (if needed)
if let Some(ticket) = ticket {
    ticket.redeem(&kv).await?;
}
```

## License

`MIT OR Apache-2.0`
