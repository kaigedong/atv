use atv_auth::{
    PageMetadata, ckey, live_signature, openapi_signature, player_auth_signature, player_ticket,
    sdk_input, token_rnd,
};
use serde_json::Value;
#[test]
fn original_component_differential_fixtures() {
    let data: Value = serde_json::from_str(include_str!("oracle-fixtures.json")).unwrap();
    let clock = data["clock"].as_u64().unwrap();
    for f in data["fixtures"].as_array().unwrap() {
        let text = |k: &str| f[k].as_str().unwrap();
        let p = &f["page"];
        let field = |k: &str| p[k].as_str().unwrap();
        let page = PageMetadata {
            url: field("url"),
            user_agent: field("user_agent"),
            referrer: field("referrer"),
            app_code_name: field("app_code_name"),
            app_name: field("app_name"),
            platform: field("platform"),
        };
        assert_eq!(
            ckey(
                text("cnlid"),
                text("ts").parse().unwrap(),
                "V1.0.0",
                text("guid"),
                "5910204",
                &page
            ),
            text("ckey")
        );
        assert_eq!(
            token_rnd("519748109", "v1", text("guid"), clock),
            text("token_rnd")
        );
        assert_eq!(
            openapi_signature(
                text("guid"),
                "www.yangshipin.cn",
                "fixture-token",
                "https:",
                "519748109",
                "fixture-sdk-input"
            ),
            text("openapi_signature")
        );
        assert_eq!(
            sdk_input(f["params"].as_object().unwrap()).unwrap(),
            text("sdk_input")
        );
        assert_eq!(
            live_signature(f["params"].as_object().unwrap()).unwrap(),
            text("live_signature")
        );
        assert_eq!(
            player_auth_signature(f["params"].as_object().unwrap()).unwrap(),
            text("auth_signature")
        );
        assert_eq!(
            player_ticket(
                text("pid"),
                text("ts"),
                text("guid"),
                "519748109",
                clock / 1000
            ),
            text("ticket")
        );
    }
}
