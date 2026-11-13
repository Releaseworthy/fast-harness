use super::*;

#[test]
fn percent_and_factor_clamp_to_50_and_400() {
    assert_eq!(factor(150), 1.5);
    assert_eq!(factor(10), 0.5);
    assert_eq!(factor(900), 4.0);
    assert_eq!(percent(1.25), 125);
    assert_eq!(percent(8.0), 400);
    assert_eq!(percent(f32::NAN), 100);
}
