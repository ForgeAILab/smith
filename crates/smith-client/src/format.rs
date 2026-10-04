//! Shared token and currency precision for session accounting.

/// Formats a token count compactly: `847`, `12.4k`, `1.2M`.
pub fn compact_tokens(value: u64) -> String {
    match value {
        0..1_000 => value.to_string(),
        1_000..1_000_000 => {
            let tenths = value / 100;
            // Drop a trailing `.0` so `12000` reads `12k`, not `12.0k`.
            if tenths.is_multiple_of(10) {
                format!("{}k", tenths / 10)
            } else {
                format!("{}.{}k", tenths / 10, tenths % 10)
            }
        }
        _ => {
            let tenths = value / 100_000;
            if tenths.is_multiple_of(10) {
                format!("{}M", tenths / 10)
            } else {
                format!("{}.{}M", tenths / 10, tenths % 10)
            }
        }
    }
}

/// Formats a micro-USD amount at Smith's established cost precision: three
/// decimal places (`$0.031`, `DESIGN.md` §7) for anything at or above a
/// tenth of a cent. Below that, three decimals would round every real,
/// nonzero spend down to the same `$0.000` a literally free session
/// renders — the dollar-figure version of the zero/unknown collapse this
/// module exists to prevent — so that range widens to full micro-USD
/// precision instead, keeping a genuine sub-cent spend visibly distinct from
/// nothing at all.
pub(crate) fn format_usd(micro_usd: u128) -> String {
    let dollars = micro_usd / 1_000_000;
    let thousandths = (micro_usd / 1_000) % 1_000;
    if micro_usd > 0 && dollars == 0 && thousandths == 0 {
        return format!("$0.{:06}", micro_usd % 1_000_000);
    }
    format!("${dollars}.{thousandths:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_are_compact_and_lose_a_pointless_decimal() {
        assert_eq!(compact_tokens(0), "0");
        assert_eq!(compact_tokens(847), "847");
        assert_eq!(compact_tokens(12_400), "12.4k");
        assert_eq!(compact_tokens(12_000), "12k");
        assert_eq!(compact_tokens(1_250_000), "1.2M");
        assert_eq!(compact_tokens(2_000_000), "2M");
        assert_eq!(compact_tokens(1_048_576), "1M");
        assert_eq!(compact_tokens(272_000), "272k");
    }
}
