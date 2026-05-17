# Chore: client-side rate limiting and 429 retry

## Why

Running the live test suite against a real NVR currently fails several
`live_read_*` tests with `Api { status: 429, code:
"TOO_MANY_REQUESTS_ERROR", message: "Too many requests" }`. The cause is
straightforward: `cargo test` runs the live integration tests in
parallel, and the NVR enforces a tight per-window quota.

A one-shot empirical check (single `GET /v1/cameras` against a real
NVR) confirmed that the server advertises its quota on **every**
response using the [RFC 9331 `RateLimit` header
fields](https://www.rfc-editor.org/rfc/rfc9331.html):

```
ratelimit-policy: "10-in-1sec"; q=10; w=1; pk=:...:
ratelimit:        "10-in-1sec"; r=9; t=1
```

That is: **10 requests per 1-second sliding window**, advertised
proactively. This is much richer than the typical `Retry-After`-only
contract — we can throttle *before* tripping 429, not just back off
after.

A follow-up burst (15 concurrent GETs) captured a real 429 response.
It carries the same `RateLimit` headers as a 200 plus a `Retry-After: 1`
header (delta-seconds form). So the server's retry hint is reliable
and can drive the reactive layer directly.

## Timing

**Do this before phase 5 (mutation endpoints).** Two reasons:

1. Phase 5 will introduce POST/PATCH against the same shared HTTP
   helpers on
   [`ProtectClient`](../crates/ferro-protect/src/client.rs#L47-L139).
   Mutations should be retried more cautiously than reads (default-off
   on writes), so the policy decision needs to land before any caller
   relies on default-on retry semantics.
2. The live test suite is currently flaky; every new `live_*` test
   added in phase 5 will make the flake worse.

## Non-negotiable: defaults must keep `cargo test --all` green

The [README's "Quick start"](../README.md#quick-start) tells users
that `cargo test --all` is safe and useful on any machine, NVR or
not. That includes a machine *with* `UNIFI_PROTECT_HOST` set — the
live tests are supposed to run cleanly under default parallelism.
They currently don't.

This chore is only complete when:

- **Both** the retry middleware and the proactive throttle are **on
  by default** when constructing a `ProtectClient` with no extra
  builder calls.
- **Every** test in the workspace — mocked tests built on `wiremock`,
  CLI e2e tests built on `assert_cmd`, AND live tests against a real
  NVR — uses the same defaults. No test file may construct the client
  with the throttle or retry disabled "to keep the test fast";
  mocked-server tests are unaffected by a 10-rps client cap (they hit
  `wiremock` on localhost which is orders of magnitude faster than
  the gate releases permits) and CLI tests already go through the
  same builder.
- `./scripts/live-test` and plain `cargo test --all` (sourced
  `.env.local`) both pass under default `cargo test` parallelism on
  a real NVR.

The "config knob to disable" exists only for the benchmark / specialty
use case. Disabling is never the default and never the test default.

If, while doing this chore, you discover the right answer is "just
serialise the live test suite with `--test-threads=1` and ship no
client-side throttle," that is **not** acceptable — it leaves
production callers (CLI, library users) exposed to the same 429s.
Document the discovery and reframe the chore, but don't close it
with a test-runner workaround.

## Scope decision (step 1 — research, not code)

The categories the client realistically needs:

| Category | Status today | This chore? |
|---|---|---|
| Retry on 429 (transient rate-limit) | none — bubbles up as `Error::Api` | yes |
| Retry on 503 (transient server) | none | yes |
| Retry on connect/read timeout | none | yes (cheap) |
| Honour `Retry-After` header | n/a | yes if header is present |
| Proactive throttling from `RateLimit` header | none | yes (the server hands us the budget; would be wasteful not to use it) |
| Per-endpoint quotas | n/a — server advertises one global policy | no |
| Cross-process/shared limiter | n/a — single client per process | no |
| Circuit breaker | none | no — out of proportion to the problem |
| Adaptive backoff from RTT | none | no |
| Concurrency cap on the live test suite | none | maybe — see "Out-of-scope adjacent items" |

The actively-felt pain is **read tests 429ing under parallel `cargo
test`**. The minimum fix is reactive retry on 429. The
*proportionate-to-effort* fix is reactive retry **plus** an adaptive
proactive throttle driven by the `RateLimit` header the server already
sends us. The proactive layer is what stops the flake from coming back
the moment we add more parallel callers (phase 5+).

### Library evaluation

Read each crate's README and pick from this shortlist; do not invent
others.

- **`reqwest-middleware` + `reqwest-retry`** — the established choice
  on paper. Wraps `reqwest::Client` as a `ClientWithMiddleware` and
  ships a `RetryTransientMiddleware`. **Verified during implementation
  that it does not honour `Retry-After`** — the built-in
  `RetryTransientMiddleware` uses its own `ExponentialBackoff` policy
  and ignores the header. That defeats the main reason to take the
  dep, so we kept `reqwest-middleware` for the middleware plumbing
  but wrote a custom retry middleware ([`src/retry.rs`](../crates/ferro-protect/src/retry.rs))
  instead of using `reqwest-retry`.

- **`backon`** — lightweight async backoff combinators (`Retryable`
  trait wrapping any async closure). No middleware layer; wrap each
  helper body with `.retry(backoff)`. Smaller dependency surface, but
  the "is this retriable?" predicate is hand-written (match on
  `Error::Api { status: 429 | 503, .. }` + connect/timeout).

- **`tower::retry`** — most powerful, also the most ceremony. Tower
  isn't already in the dependency tree; introducing it for this alone
  is over-budget.

- **DIY in `client.rs`** — wrap each helper with a small `with_retry`
  closure plus a `should_retry(&Error) -> Option<Duration>` predicate.
  Maybe 60 lines. No new dependency. Reasonable if the proactive
  throttle is built DIY too (see below) and we'd otherwise be taking
  the dep for retry alone.

For the **proactive throttle** layer:

- **`governor`** — well-known token-bucket / GCRA rate limiter.
  Configure with the advertised `q` / `w` (e.g. 10 per 1 second).
  Static config is straightforward. Updating the bucket dynamically
  from `RateLimit` headers is *possible* but not idiomatic (`governor`
  doesn't expose runtime quota mutation cleanly).

- **`tower::limit::rate`** — same Tower-stack objection as for retry.

- **DIY semaphore-style limiter** — a `tokio::sync::Semaphore` sized
  to the server's `q`, with permits released on a sliding-window
  schedule, gives us full control to update from `RateLimit` headers
  on every response. ~80 lines. The combination of "we already need to
  parse the response header" and "we want dynamic capacity" tips the
  balance away from `governor`'s static config.

**Default recommendation:**

- **Retry**: `reqwest-middleware` + `reqwest-retry`. The "respect
  `Retry-After`" logic is non-trivial enough that copying the crate's
  implementation into our tree is silly; the dependency cost is
  acceptable.
- **Proactive throttle**: DIY in `client.rs`, semaphore-backed,
  reading `RateLimit` from each response and adjusting capacity. The
  full surface fits in ~100 lines and we *will* need the dynamic
  behaviour.

If, during step 2's empirical pass, the 429 response turns out to
*not* carry `Retry-After`, downgrade the retry layer to `backon` (or
DIY) — `reqwest-retry`'s main differentiator is `Retry-After`, and
without that we're paying for a feature we don't use.

### Out-of-scope adjacent items (deliberately deferred)

- **Reducing live test parallelism via
  `scripts/live-test`.** Out of *this* chore — but document in
  `PROGRESS.md` as a known mitigation if a contributor hits 429s
  before this lands. The proper fix is client-side, not test-runner-
  side; test-runner serialisation papers over the client gap.
- **Retry on mutations (POST/PATCH/DELETE) by default.** Out — phase 5
  may revisit. Default-off; expose an opt-in builder flag.
- **Circuit breaker / open-circuit fallback.** Out — the failure mode
  here is "transient rate limit," not "server down for 10 minutes."
  Adding a breaker is solving a problem we don't have.
- **Per-endpoint quotas / token bucket sharding.** Out — server
  advertises a single global policy.
- **Sharing the rate limiter across multiple `ProtectClient`
  instances.** Out — the API contract is "one client per process."
  Two clients in the same process is a foot-gun the docs should
  discourage rather than the limiter accommodate.
- **Adaptive backoff based on observed RTT or success rate.** Out —
  server tells us its budget; no need to infer.
- **Exposing rate-limit telemetry as a public API
  (e.g. `client.rate_limit_state()`).** Out for now. Reconsider only
  if a user files an issue asking for it.

## Action items (assuming the recommendation stands)

### 2. Empirical: capture a real 429 response

Before locking in the design, capture the headers of an actual 429
response from the NVR (the success-path probe in step 0 only saw a
200). The cheapest way: run the existing live test suite — it already
trips 429s under parallel `cargo test` — with
`RUST_LOG=ferro_protect=debug` and `--nocapture`, and have a debug
log line in `json_response` print response headers on the non-success
path.

What we need to know:

- Does the 429 response include `Retry-After`?
- Does it include the `RateLimit` headers, or only `RateLimit-Policy`?
- What's the value of `t` (reset window) in practice — is it always
  `1` or does it grow when you're sustained-over?

Record findings in `PROGRESS.md` and update the recommendation in
this file if the answers change the library choice.

### 3. Add dependencies

In root [`Cargo.toml`](../Cargo.toml) under `[workspace.dependencies]`:

```toml
reqwest-middleware = { version = "<latest>", default-features = false }
reqwest-retry      = { version = "<latest>", default-features = false }
```

(Pin specific versions during step 3; `<latest>` is a placeholder.
Run `cargo deny check` against the new transitive tree before
committing — `reqwest-middleware` brings in `anyhow` and a few
others.)

In [`crates/ferro-protect/Cargo.toml`](../crates/ferro-protect/Cargo.toml):

```toml
reqwest-middleware = { workspace = true }
reqwest-retry      = { workspace = true }
```

The CLI crate does not need either dep directly.

### 4. Switch `ProtectClient::http` to `ClientWithMiddleware`

In [`crates/ferro-protect/src/client.rs`](../crates/ferro-protect/src/client.rs):

- Change the field type `http: reqwest::Client` to
  `http: reqwest_middleware::ClientWithMiddleware`.
- In `ProtectClientBuilder::build`, wrap the built `reqwest::Client`
  with `ClientBuilder::new(http).with(retry_middleware).build()`.
- Configure `RetryTransientMiddleware::new_with_policy(
  ExponentialBackoff::builder()
    .retry_bounds(initial, max)
    .build_with_max_retries(max_retries))`.

Every existing helper (`get_json`, `post_json`, `patch_json`,
`send_no_content`, `get_bytes`) keeps the same call shape — the
middleware client exposes the same `.get()`, `.post()`, `.send()`
fluent API.

### 5. Add the dynamic proactive throttle

This is the part with no off-the-shelf answer.

Add a new module `crates/ferro-protect/src/rate_limit.rs` containing:

```rust
pub(crate) struct AdaptiveLimiter {
    // capacity: current view of the server's `q`
    // permits: tokio::sync::Semaphore sized to capacity
    // refill_task: background task that releases permits on the `w` cadence
}

impl AdaptiveLimiter {
    pub(crate) fn new(initial_capacity: u32, window: Duration) -> Self { ... }

    /// Block until a permit is available, return it as a guard.
    pub(crate) async fn acquire(&self) -> Permit { ... }

    /// Parse `RateLimit` / `RateLimit-Policy` headers from a response
    /// and adjust capacity if they advertise a different `q`/`w` than
    /// what we currently track. No-op when headers are absent.
    pub(crate) fn observe(&self, headers: &HeaderMap) { ... }
}
```

Wire it into `client.rs` by calling `self.limiter.acquire().await`
at the top of each helper (before the `reqwest` call) and
`self.limiter.observe(response.headers())` after each response (on
both success and error paths — the server sends the headers either
way). The acquire/observe pair is the only edit in the helpers
themselves; the rest of each helper stays one line of `reqwest` and
one line of response handling.

Default initial capacity: `10`, window: `1s`. These are what the
server advertises today (`10-in-1sec`). If a future firmware
advertises a different policy on the first response, `observe()`
adjusts.

### 6. Builder knobs

Add to `ProtectClientBuilder`. **All defaults active** — see the
non-negotiable section above. The knobs exist to turn things *down*
for the rare benchmark or "I have a different NVR with a higher
quota" case, not to gate the feature.

```rust
.max_retries(u32)                        // default 3
.retry_initial_backoff(Duration)         // default 200ms
.retry_max_backoff(Duration)             // default 5s
.retry_on_mutations(bool)                // default false (idempotent reads only)
.rate_limit(Option<RateLimitConfig>)     // default Some(default 10/1s) — None disables proactive throttling
```

`RateLimitConfig` is a small public struct exposing `initial_capacity`
and `window`. Default sets `10 / 1s` to match the observed server
policy, but users can override (e.g. lower it on a busy NVR, or set
`None` to opt out entirely for benchmarking).

`retry_on_mutations(false)` means the retry middleware only retries
GETs by default. We get that by configuring the middleware's
`Retryable::is_retryable` predicate to inspect the request method
and skip non-idempotent verbs. If the user opts in, retries apply
uniformly.

Document each knob with the same rigour as the existing
[`TlsMode`](../crates/ferro-protect/src/client.rs#L27-L34) doc
comment.

### 7. Tests

- **Unit:** in `rate_limit.rs`, test that `observe()` parses
  RFC 9331 headers correctly (the example values from this doc are
  fine fixtures) and adjusts capacity. Test that absent headers
  are a no-op.
- **Mocked integration:** add `crates/ferro-protect/tests/rate_limit.rs`
  with a `wiremock` server that:
  - Returns 429 + `Retry-After: 1` on the first call and 200 on the
    second; assert the client succeeds and that exactly two requests
    were observed.
  - Returns 200 + `ratelimit: "5-in-1sec"; r=0; t=1`; assert the next
    `acquire()` blocks for ~1s.
  - Returns 429 four times in a row with `max_retries=3`; assert the
    client surfaces `Error::Api { status: 429, .. }`.
- **Don't slow the rest of the suite.** All other mocked tests in
  [`crates/ferro-protect/tests/`](../crates/ferro-protect/tests/) and
  [`crates/ferro-protect-cli/tests/`](../crates/ferro-protect-cli/tests/)
  must continue to run at full speed. They will, because they hit
  `wiremock` on localhost which responds in microseconds — the 10/s
  default never throttles a single mocked request. If you find a
  mocked test that *does* feel slow after this change, the right fix
  is to check why the test is making >10 requests in a tight loop,
  not to disable the limiter.
- **Live:** *do not* add a dedicated live test for rate-limit. The
  existing `live_read_*` suite IS the rate-limit test — if it stops
  flaking after this lands, we're done. Add a `PROGRESS.md` entry
  noting the change in baseline.

### 8. Docs

- [`ARCHITECTURE.md`](../ARCHITECTURE.md) — file map gains `rate_limit.rs`;
  the "client architecture" section gains a paragraph on the
  middleware layer + proactive throttle.
- [`PLAN.md`](../PLAN.md) — phase 5+ wrapper template stays unchanged
  (helpers handle it transparently), but note in the phase 5 intro
  that mutation methods default to non-retried.
- [`README.md`](../README.md) — short note under "Running tests" that
  live tests no longer require `--test-threads=1` (or, if step 7
  shows otherwise, document the residual cap).
- No CLI surface change.

### 9. Verify, log, commit

`cargo fmt --all -- --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test --all`, `cargo deny
check`. All four green. Then run `./scripts/live-test` and verify
the previously-flaking `live_read_*` tests pass cleanly under
default parallelism.

`PROGRESS.md` entry per the canonical logging format. Commit:

```
feat(client): adaptive rate limiting and 429 retry middleware
```

If the chore lands as multiple commits (likely — retry middleware,
limiter module, builder knobs, tests), follow the per-phase
one-logical-step-per-commit pattern already in use.

## Acceptance criteria

- `ProtectClient` retries 429 and 503 transparently up to a
  configurable bound, honouring `Retry-After` when present.
- `ProtectClient` reads the RFC 9331 `RateLimit` headers from every
  response and adapts an internal throttle so the suite of in-process
  callers cannot exceed the server's advertised budget.
- **Both retry and proactive throttle are on by default.** A user
  calling `ProtectClient::builder().host(...).api_key(...).build()`
  with no other knob calls gets both behaviours.
- Retries default to **GET only**; mutations require an opt-in
  builder flag.
- **`cargo test --all` is green** with a sourced `.env.local`
  (i.e. live tests pointed at a real NVR) under default `cargo test`
  parallelism, **without** any `--test-threads=N` cap. This is the
  primary acceptance signal — the README's "Quick start" promise
  holds.
- `./scripts/live-test` passes against a real NVR.
- All four gates green.
- No new dependency in the CLI crate. Rate limiting is a library
  concern.
- `PROGRESS.md` carries the step-2 empirical findings and the
  step-9 baseline-change note.

## Out of scope

- **CLI flags for rate-limit tuning.** Library-only knobs for now.
  Add CLI surface only if a user actually needs to tune it from the
  command line.
- **Per-endpoint or per-resource rate limiting.** Server advertises a
  single global policy.
- **Cross-process or distributed rate limiting.**
- **Circuit breakers, bulkheads, or any wider resilience pattern.**
- **Public observability API for rate-limit state.** Logs are
  sufficient for now (debug-level when the throttle blocks, info-level
  when capacity adjusts).
- **Changes to mutation semantics.** Phase 5 introduces mutation
  endpoints; this chore only adds the *infrastructure* (default-off
  retry-on-write knob).
