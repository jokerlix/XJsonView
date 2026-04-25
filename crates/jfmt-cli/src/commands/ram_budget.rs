//! Pre-flight RAM budget for `--materialize` modes. Used by both
//! `commands::filter` (M4b) and `commands::validate` (M5).

/// Estimate peak RAM for materializing `input`. Returns `None` when
/// the input is stdin or its size can't be determined — callers
/// interpret `None` as "skip the check" per spec D3.
pub(super) fn estimate_peak_ram_bytes(spec: &jfmt_io::InputSpec) -> Option<u64> {
    let path = spec.path.as_ref()?;
    let meta = std::fs::metadata(path).ok()?;
    let on_disk = meta.len();
    // Effective compression: explicit override, then file extension.
    let compression = spec
        .compression
        .unwrap_or_else(|| jfmt_io::Compression::from_path(path));
    let multiplier: u64 = match compression {
        jfmt_io::Compression::None => 6,
        jfmt_io::Compression::Gzip | jfmt_io::Compression::Zstd => 5 * 6, // 30
    };
    Some(on_disk.saturating_mul(multiplier))
}

/// Pure predicate: is `estimate` under 80% of `total_ram`?
pub(super) fn budget_ok(estimate: u64, total_ram: u64) -> bool {
    // 80% = total_ram * 4 / 5. Compute as `total_ram / 5 * 4` to
    // reduce overflow risk on very large `total_ram` values.
    estimate < total_ram / 5 * 4
}

/// Query the actual system total RAM. Wraps sysinfo per spec Annex B.
pub(super) fn system_total_ram_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use super::budget_ok;

    #[test]
    fn budget_ok_under_80_percent() {
        assert!(budget_ok(1 << 30, 2u64 << 30));
    }

    #[test]
    fn budget_not_ok_over_80_percent() {
        let total = 2u64 << 30;
        let estimate = total * 85 / 100;
        assert!(!budget_ok(estimate, total));
    }

    #[test]
    fn budget_ok_at_zero_estimate() {
        assert!(budget_ok(0, 1 << 30));
    }
}
