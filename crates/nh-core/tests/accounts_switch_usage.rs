use nh_core::accounts::usage::usage_from_headers;

#[test]
fn test_usage_from_headers() {
    let headers = vec![
        (
            "anthropic-ratelimit-unified-5h-utilization".to_string(),
            "0.5".to_string(),
        ),
        (
            "anthropic-ratelimit-unified-5h-reset".to_string(),
            "1700000000".to_string(),
        ),
    ];
    let usage = usage_from_headers(&headers).unwrap();
    let five_hour = usage.five_hour.as_ref().unwrap();
    assert_eq!(five_hour.utilization, 50.0);
    assert_eq!(five_hour.resets_at, Some(1700000000));
}
