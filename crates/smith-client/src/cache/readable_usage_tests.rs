use super::*;
#[test]
fn reported_cache_reads_do_not_require_a_controllable_cache() {
    let line = render_cache_read_usage(Some(72), Some(30_400)).unwrap();
    assert!(line.contains("72% of input read from cache"));
    assert!(line.contains("30.4k"));
    assert!(!line.contains("unsupported"));
    assert!(!line.contains('?'));
}
#[test]
fn missing_and_zero_cache_usage_are_distinct() {
    assert_eq!(render_cache_read_usage(None, None), None);
    assert!(
        render_cache_read_usage(Some(0), Some(0))
            .unwrap()
            .contains("0%")
    );
    assert!(
        render_cache_read_usage(None, Some(0))
            .unwrap()
            .contains("0 cached")
    );
}
