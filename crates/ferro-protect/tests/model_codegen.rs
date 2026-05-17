#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Focused smoke tests for the generated-model seam.
//!
//! Two things this file guards:
//!
//! 1. **Surface fingerprint.** `_seam_signatures` names every type the
//!    library re-exports from typify-generated code. A spec bump that
//!    renames or removes any of them fails compilation here before it
//!    can fail in a wrapper module.
//! 2. **Derive fingerprint.** `_assert_derives` asserts the trait set
//!    every re-exported model is expected to provide. A typify update
//!    that drops `PartialEq` (or any other derive) trips a compile error
//!    here instead of surfacing as a confusing breakage later.

use ferro_protect::models::{ApplicationInfo, Camera, CameraId, Chime, ChimeId, ProtectVersion};
use serde::{Deserialize, Serialize};

const FIXTURE_INFO: &str = include_str!("fixtures/info_ok.json");

#[test]
fn application_info_round_trips() {
    let info: ApplicationInfo = serde_json::from_str(FIXTURE_INFO).expect("fixture parses");
    assert_eq!(info.application_version.to_string(), "6.2.83");

    let encoded = serde_json::to_string(&info).expect("serializes");
    let reparsed: ApplicationInfo = serde_json::from_str(&encoded).expect("re-parses");
    assert_eq!(reparsed, info);
}

#[test]
fn generated_id_newtypes_are_stable() {
    let camera_id = CameraId::from("camera-1".to_string());
    let chime_id = ChimeId::from("chime-1".to_string());

    assert_eq!(camera_id.to_string(), "camera-1");
    assert_eq!(chime_id.to_string(), "chime-1");
}

#[test]
fn id_newtypes_round_trip_through_json() {
    // String-shaped ids must encode as a JSON string (not a struct), so
    // wrappers can interpolate them into URLs and bodies unchanged.
    let encoded = serde_json::to_string(&CameraId::from("abc".to_string())).expect("encodes");
    assert_eq!(encoded, "\"abc\"");
    let decoded: CameraId = serde_json::from_str("\"abc\"").expect("decodes");
    assert_eq!(decoded.to_string(), "abc");
}

// Compile-only assertions: every re-exported type must satisfy the
// trait set wrappers and consumers rely on. If typify ever stops
// emitting one of these derives this stops compiling.
fn _assert_derives() {
    fn assert_model<
        T: Serialize + for<'de> Deserialize<'de> + Clone + std::fmt::Debug + PartialEq,
    >() {
    }
    assert_model::<ApplicationInfo>();
    assert_model::<Camera>();
    assert_model::<CameraId>();
    assert_model::<Chime>();
    assert_model::<ChimeId>();
    assert_model::<ProtectVersion>();
}

// Compile-only fingerprint of the seam: name every re-exported type in
// a signature so a rename in `models.rs` (the seam) or in the typify
// output it pulls from fails here before reaching wrapper code.
#[allow(dead_code)]
fn _seam_signatures(
    _info: ApplicationInfo,
    _camera: Camera,
    _camera_id: CameraId,
    _chime: Chime,
    _chime_id: ChimeId,
    _version: ProtectVersion,
) {
}
