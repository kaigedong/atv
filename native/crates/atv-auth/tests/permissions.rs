use atv_auth::{ResolveError, parse_live_info};
use serde_json::json;

#[test]
fn quality_choices_only_include_advertised_video_definitions() {
    let response = json!({"code":0,"data":{"iretcode":0,"playurl":"https://fixture.invalid/live.m3u8","defn":"hd",
        "formats":[{"fn":"hd","defnrate":"540P"},{"fn":"audio"},{"fn":"shd"},{"fn":"shd"},{"fn":"fhd"},{"fn":"unknown"}]}});
    let info = parse_live_info(&response).unwrap();
    assert_eq!(info.definitions(), ["hd", "shd", "fhd"]);
    assert_eq!(info.definition_height(), Some(540));
}
#[test]
fn rejection_never_yields_a_url() {
    for code in [5, 12, 25, 45, 46, 47, 53, 54, 56, 58, 67] {
        let response = json!({"code":0,"data":{"iretcode":code,"playurl":"https://fixture.invalid/live.m3u8?token=fixture"}});
        assert!(
            matches!(parse_live_info(&response),Err(ResolveError::Server{iretcode:Some(c),..}) if c==code)
        );
    }
    assert!(matches!(
        parse_live_info(&json!({"ret":10012,"msg":"access denied"})),
        Err(ResolveError::Server { code: 10012, .. })
    ));
}
#[test]
fn preview_and_attempt_limits_survive_parsing() {
    let response = json!({"code":0,"data":{"iretcode":0,"playurl":"https://fixture.invalid/live.m3u8?token=fixture","defn":"hd","playtime":60,"totalplaytime":180,"restpreviewcnt":1,"previewcnt":3,"playback":{"playbacktime":0},"formats":[],"extended_param":"&fixture=1"}});
    let info = parse_live_info(&response).unwrap();
    assert_eq!(info.preview_seconds, 60);
    assert_eq!(info.preview_total_seconds, 180);
    assert_eq!(info.attempts_remaining, 1);
    assert_eq!(info.attempts_total, 3);
    assert!(!format!("{info:?}").contains("token=fixture"));
}
#[test]
fn remote_javascript_is_not_executed() {
    let response = json!({"code":0,"data":{"iretcode":0,"playurl":"https://fixture.invalid/live.m3u8?token=fixture","chanll":"{\"code\":\"fixture\"}"}});
    assert!(matches!(
        parse_live_info(&response),
        Err(ResolveError::UnsupportedChallenge)
    ));
}
#[test]
fn signed_query_octets_are_preserved() {
    let response = json!({"code":0,"data":{"iretcode":0,"playurl":"https://fixture.invalid/live.m3u8?signature=a%2fb%2Bc&token=x%20y","extended_param":"&opaque=%2f%2B","defn":"hd"}});
    let info = parse_live_info(&response).unwrap();
    assert_eq!(
        info.url(),
        "https://fixture.invalid/live.m3u8?signature=a%2fb%2Bc&token=x%20y&revoi=&opaque=%2f%2B"
    );
    for value in [
        "https://fixture.invalid/live#fragment",
        "https://user:password@fixture.invalid/live",
        "http://fixture.invalid/live",
    ] {
        let response = json!({"code":0,"data":{"iretcode":0,"playurl":value}});
        assert!(matches!(
            parse_live_info(&response),
            Err(ResolveError::Malformed { stage: "playurl" })
        ));
    }
}
#[test]
fn pre_cancelled_and_invalid_requests_do_not_contact_service() {
    use atv_auth::{LiveResolver, ResolverConfig};
    use std::sync::atomic::AtomicBool;
    let resolver = LiveResolver::new(ResolverConfig::default()).unwrap();
    assert!(matches!(
        resolver.resolve_quality_cancellable(
            "600001859",
            "2024078201",
            "hd",
            1,
            &AtomicBool::new(true)
        ),
        Err(ResolveError::Cancelled)
    ));
    assert!(matches!(
        resolver.resolve_live("../../fixture", "2024078201"),
        Err(ResolveError::Malformed {
            stage: "channel_identifier"
        })
    ));
}
