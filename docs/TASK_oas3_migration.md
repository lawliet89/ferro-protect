# Chore: migrate codegen from progenitor to oas3 + typify (models only)

## Why

The 7.1.60 spec analysis showed the current rewrite layer in `build.rs` will need more cases to handle the new spec — WebSocket `oneOf`→`$ref` indirection, `anyOf` + numeric `const` patterns for sirens, more explicit `409`/`503` responses, new device families adding more discriminated unions. The rewrite layer was sustainable for one or two spec bumps; 7.1.60 is the version where it stops paying for itself.

The fix is to keep codegen for the *models* (where it pays — typify handles JSON Schema directly, including most 3.1 constructs) and drop codegen for the *HTTP client* (where it costs — progenitor's strictness about operationIds, `default` responses, `allOf` shapes, etc. is what drives most of the rewrite rules). Wrappers like `CamerasApi<'a>` were always going to be hand-written; in the new world they call `reqwest::Client` directly instead of calling progenitor's generated client which then calls `reqwest::Client`. The intermediate layer disappears.

Net result: the rewrite layer shrinks dramatically, the WebSocket schema change in 7.1.60 becomes a non-event (`deviceEvent` is just a named schema typify generates a struct for), and the public API surface — `models::*` as the single seam between codegen and hand-written code — gets *stronger* not weaker as a forward-compatibility primitive.

## Timing

**Do this chore before phase 3.** Right now only the `info` endpoint depends on progenitor's generated client; by phase 5 there will be 14+ wrappers all depending on progenitor's method signatures and the migration cost will be considerably higher. Phase 3 (the API key loader) doesn't touch the library's HTTP layer at all, so it doesn't matter whether this chore lands before or after phase 3 from a dependency standpoint — but earlier is cheaper.

## Logging and commit policy

Follow the rules in `PLAN.md` exactly:

- "Operating instructions for you (Claude Code) → Progress logging" — use the timestamp format `YYYY-MM-DD HH:MM ±HHMM` captured at write time (do not copy the placeholder), and commit the progress entry with the *next* phase's work.
- "Operating instructions for you (Claude Code) → Commit policy" — one chore, one commit, Conventional Commits style.
- "Operating instructions for you (Claude Code) → PGP signing — important" — never bypass signing; hand the commit back to the user if it prompts.
- "Operating instructions for you (Claude Code) → Guardrails" — fmt, clippy, test, deny all green before commit.

Commit message: `chore(codegen): migrate from progenitor to oas3 + typify for models-only generation`

## Action items

### 1. Dependency changes

In `crates/ferro-protect/Cargo.toml`:

- **Remove from `[build-dependencies]`**: `progenitor`, anything brought in only to satisfy progenitor.
- **Remove from `[dependencies]`**: `progenitor-client`. The downstream-runtime helpers from progenitor are no longer needed.
- **Remove from workspace `[workspace.dependencies]` if unused elsewhere**: `openapiv3` (oas3 replaces it).
- **Add to `[build-dependencies]`**: `oas3` (latest), `typify` (latest). Keep `serde_json`, `syn`, `prettyplease`.

Verify nothing else in the workspace references the removed crates before deleting their workspace entries. `cargo tree` is your friend.

### 2. Rewrite the codegen pipeline

The new `build.rs` flow (or, more likely, a slim `build.rs` that delegates to `build_support/spec_rewrite.rs` — keep the existing extraction pattern):

1. `println!("cargo:rerun-if-changed={path}")` for the spec file.
2. Read and parse the spec. Two viable parsing options — pick whichever is simpler in practice:
   - **Option A (preferred):** parse with `oas3` for validation + type-safe traversal. Extract `components.schemas`. For each schema, serialize back to `serde_json::Value` to hand to typify.
   - **Option B:** skip `oas3` entirely; parse the spec with `serde_json::Value`, walk to `components.schemas`, hand each schema directly to typify. Simpler if oas3's type model adds friction during extraction.

   If you go with B, then `oas3` shouldn't be a dependency. Decide and document in the PROGRESS entry.
3. For each schema under `components.schemas`, feed it to `typify::TypeSpace::add_type` (or whichever API the current typify exposes). Configure typify to derive `serde::{Serialize, Deserialize}`, `Debug`, `Clone`, and `PartialEq` where reasonable.
4. Convert the resulting `TokenStream` to a string via `prettyplease`. Write it to `$OUT_DIR/generated.rs`.
5. Done. No path/operation generation. No response enum generation. No client struct generation.

### 3. Reduce the rewrite layer

The `spec_rewrite::rewrite` function currently handles six categories of rewrites. After this chore, evaluate each:

- **3.1 → 3.0 nullable conversion** — likely still needed; typify accepts JSON Schema but some constructs (`type: ["string", "null"]`) may still cause issues. Test against the real spec before removing.
- **Synthesize operationIds** — drop. We no longer generate operations.
- **`default` → `4XX` rename** — drop. We no longer generate response enums.
- **Flatten `allOf: [<single $ref>]`** — drop unless typify still complains. Test first.
- **Strip `additionalProperties: false` from combinators** — drop unless typify still complains. Test first.
- **Strip multipart, rewrite `image/jpeg` → octet-stream** — drop entirely. These were progenitor's request/response body shape concerns; typify only sees schemas.

Run typify against the unmodified-except-for-nullable spec and see what survives. Most rewrites should fall away. Update `UPGRADING.md`'s description of the rewrite layer accordingly — the section explaining "what the rewrite functions do" will be shorter.

### 4. Reshape the snapshot test

`tests/spec_rewrite_snapshot.rs` currently snapshots ~5,590 lines of rewritten OpenAPI. After this chore, that JSON is much smaller (potentially nearly identical to the input spec). Two reasonable directions:

- **Keep it, narrowed**: snapshot whatever the trimmed `rewrite()` function still produces. If it ends up being essentially a passthrough, the snapshot is small and cheap — fine.
- **Drop it, replace with a generated-models snapshot**: snapshot the generated Rust source from typify instead. This has higher signal (catches type renames, derive changes, enum variant shifts) but the snapshot is large.

Recommended: drop the spec-rewrite snapshot, add a much smaller test that asserts a representative subset of generated types compile and round-trip. Decide and document.

### 5. Rewrite the HTTP client layer

`crates/ferro-protect/src/client.rs` currently wraps a progenitor `Client`. After:

```rust
pub struct ProtectClient {
    http: reqwest::Client,
    base_url: Url,
}
```

Add a private async helper for the common GET-JSON pattern, so future endpoint methods (phases 4-9) don't repeat themselves:

```rust
impl ProtectClient {
    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> { ... }
    async fn post_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> { ... }
    async fn patch_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> { ... }
    async fn get_bytes(&self, path: &str) -> Result<Bytes> { ... }
    // ...etc
}
```

These helpers do error mapping (deserialize the spec's error body shape on non-2xx, map to `Error::Api`), URL construction (`self.base_url.join(path)`), and any tracing spans. Phase 4 onward gets a one-line implementation per endpoint:

```rust
pub async fn info(&self) -> Result<ApplicationInfo> {
    self.get_json("/v1/meta/info").await
}
```

### 6. Re-port the `info` endpoint as the validation case

Phase 2's `info()` is the only existing endpoint. Port it to the new helper. Confirm:

- `tests/info.rs` (wiremock) still passes unchanged — the public API surface is identical.
- `tests/live.rs::live_read_info` still passes when env is configured.
- `crates/ferro-protect-cli/tests/info.rs` (assert_cmd) still passes unchanged.

The fact that the existing tests pass without modification is the proof the migration didn't regress public behavior.

### 7. Error layer cleanup

`Error::from_progenitor` and any progenitor-specific error mapping disappears. The remaining error mapping path is now uniform: every helper in step 5 calls a single `map_error_response()` that:

1. Checks status code; on 2xx, returns the deserialized body.
2. On non-2xx, attempts to deserialize the body as the spec's error shape (`{ name, error }` or whatever 7.1.60 lands on).
3. Returns `Error::Api { status, code, message }` on success; falls back to `Error::Api { status, code: "unknown", message: <raw_body_truncated> }` if deserialization fails.

This is simpler than the progenitor mapping and doesn't require a generic `E: Serialize` parameter.

### 8. Update PLAN.md

The following sections in `PLAN.md` reference progenitor and need editing — keep the edits minimal and surgical:

- **Phase 1 ("Codegen pipeline")** — rewrite the task list to reflect the new pipeline (oas3 + typify, models only). The fallback options paragraph at the end (openapi-generator-cli, hand-written types) becomes mostly moot; replace with a shorter note: "if typify chokes on a specific schema, hand-write that one type in `models.rs` and skip it during codegen via a small allowlist in `build.rs`."
- **Phase 2 step 3 ("`crates/ferro-protect/src/client.rs`")** — change "Wraps the progenitor-generated client" to "Builds on a hand-written `reqwest::Client` wrapper with shared get/post/patch helpers."
- **Phase 2 step 4** — drop "Map any progenitor errors into `unifi_protect::Error`" since the indirection is gone.
- **Forward-compatibility section, item 3 ("Wrappers delegate, they do not re-implement")** — rewrite. Wrappers now *are* the implementation; the principle becomes "wrapper methods are one-liners delegating to the shared HTTP helpers, returning `models::Foo`. Never construct request URLs or bodies inline in business logic — keep that mechanical layer in the helpers so future endpoints stay uniform."
- **Reference: spec source** — update the "Format: OpenAPI 3.1.0 (needs 3.0 down-conversion for progenitor; see phase 1)" line to drop the progenitor reference. Replace with "Format: OpenAPI 3.1.0; consumed natively by oas3 with minor JSON Schema preprocessing for typify (see phase 1)."

Don't refactor more than necessary. The phase ordering, testing strategy, API key loader, and everything below phase 3 stays exactly as written.

### 9. Update UPGRADING.md

Adjust the failure-triage section. The new failure modes for a future spec bump are different:

- Typify failing on a schema construct → add a preprocessing rule or hand-write the type.
- A new endpoint without a wrapper → expected; the agent adds a one-line wrapper.
- A renamed schema → fix in `models.rs` (unchanged).

Strike the parts about progenitor-specific failure modes (operationId conflicts, `default` response handling, etc.).

### 10. Verify, log, commit

Run fmt, clippy, test, deny. All four must be green. Append a PROGRESS.md entry following the format in PLAN.md, including the timestamp in `±HHMM` form captured at write time. Commit.

## Acceptance criteria

- `crates/ferro-protect/Cargo.toml` no longer depends on `progenitor` or `progenitor-client`.
- `build.rs` generates the `models` module via typify; no operation/client/error generation happens.
- The rewrite layer in `build_support/spec_rewrite.rs` is materially smaller than before (most rewrite functions removed or proven unneeded).
- `ProtectClient` is a hand-written wrapper around `reqwest::Client` with shared get/post/patch/bytes helpers.
- `ProtectClient::info()` works against both the wiremock mock and a real NVR (live test passes when env is configured).
- All existing tests pass without modification to their *public* surface (test bodies may need small import path updates but assertion logic stays the same).
- PLAN.md and UPGRADING.md are updated where they reference progenitor.
- PROGRESS.md has a chore entry per the canonical logging format.
- fmt, clippy, test, deny all green.

## Out of scope

- **No new endpoints.** This chore migrates the codegen substrate; it does not add cameras/lights/etc. Those are still phase 4's job.
- **No 7.1.60 spec bump.** This chore stays on 6.2.83. The 7.1.60 upgrade happens separately via `scripts/update-spec` after this chore lands; that's when the value of the migration becomes visible.
- **No changes to phases 3-10.** The plan structure, ordering, and testing strategy are untouched.
- **No WebSocket work.** Phase 9 still implements WS by hand; this chore doesn't pre-empt it. The observation that `deviceEvent` becomes a non-event under the new world is a *future* benefit, not something to implement now.
- **No insta snapshot of generated Rust source unless step 4 explicitly takes that direction.** Keep snapshot scope narrow per PLAN.md's testing strategy.
