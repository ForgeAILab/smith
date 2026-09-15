//! Human-facing wall-clock timestamps. Wire/persistence timestamps stay numeric.
//!
//! Resolve the offset at the represented instant, not today's offset: a saved
//! winter session must not move by an hour when viewed during daylight time.
//! Chrono's local zone lookup also works in Smith's multithreaded Tokio host.

use chrono::{DateTime, Local, Utc};

/// Resolve a Unix-millisecond instant in the operating system's local zone.
/// The explicit offset in every rendered timestamp makes its meaning clear.
pub fn local_timestamp(millis: u64) -> String {
    let Some(utc) = i64::try_from(millis)
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
    else {
        return "invalid timestamp".to_owned();
    };
    utc.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %:z")
        .to_string()
}

/// Local offset at a particular instant, for relative session-picker labels.
pub fn local_offset_at(millis: u64) -> Option<time::UtcOffset> {
    let utc = DateTime::<Utc>::from_timestamp_millis(i64::try_from(millis).ok()?)?;
    time::UtcOffset::from_whole_seconds(utc.with_timezone(&Local).offset().local_minus_utc()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn unix_milliseconds_are_an_instant_not_a_duration() {
        let utc = DateTime::<Utc>::from_timestamp_millis(1_789_499_999_019).unwrap();
        let toronto_summer = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(
            utc.with_timezone(&toronto_summer)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string(),
            "2026-09-15 15:19:59 -04:00"
        );
        assert!(!local_timestamp(1_789_499_999_019).ends_with("ms"));
    }

    #[test]
    fn out_of_range_timestamp_does_not_overflow_or_panic() {
        assert_eq!(local_timestamp(u64::MAX), "invalid timestamp");
        assert_eq!(local_offset_at(u64::MAX), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_offset_is_available_inside_the_multithreaded_host() {
        assert!(local_offset_at(1_789_499_999_019).is_some());
        let rendered = local_timestamp(1_789_499_999_019);
        assert_ne!(rendered, "invalid timestamp");
        assert!(chrono::DateTime::parse_from_str(&rendered, "%Y-%m-%d %H:%M:%S %:z").is_ok());
    }

    #[test]
    fn named_local_zone_handles_daylight_saving_on_worker_threads() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "time_display::tests::named_zone_probe",
                "--nocapture",
            ])
            .env("TZ", "America/Toronto")
            .env("SMITH_TIME_ZONE_TEST", "1")
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn named_zone_probe() {
        if std::env::var("SMITH_TIME_ZONE_TEST").as_deref() != Ok("1") {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        runtime.block_on(async {
            let rendered = tokio::spawn(async {
                (
                    local_timestamp(1_789_499_999_019),
                    local_timestamp(1_768_504_799_019),
                )
            })
            .await
            .unwrap();
            assert_eq!(rendered.0, "2026-09-15 15:19:59 -04:00");
            assert_eq!(rendered.1, "2026-01-15 14:19:59 -05:00");
        });
    }
}
