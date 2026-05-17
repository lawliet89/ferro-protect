#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Focused smoke tests for the generated-model seam.

use ferro_protect::models::{ApplicationInfo, CameraId, ChimeId};

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
