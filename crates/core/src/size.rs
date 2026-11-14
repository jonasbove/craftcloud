//! Sizes as people read them: decimal units (1 KB = 1000 bytes), as file managers and the rest of
//! CraftCloud show them. One formatter, so a quota and a refusal never disagree (T-049).

/// `1234567` → `1.2 MB`; under 1000 bytes, `999 B`.
pub fn text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 { format!("{value:.1} {}", UNITS[unit]) } else { format!("{value:.0} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decimal_units() {
        assert_eq!(super::text(999), "999 B");
        assert_eq!(super::text(1_234), "1.2 KB");
        assert_eq!(super::text(1_000_000_000), "1.0 GB");
    }
}
