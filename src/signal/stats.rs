use crate::util::pct;

/// z-score of `x` against `reference` (sample standard deviation). `None` with fewer than
/// `min_n` reference points or a flat reference.
pub fn zscore(x: f64, reference: &[f64], min_n: usize) -> Option<f64> {
    if reference.len() < min_n.max(2) {
        return None;
    }
    let n = reference.len() as f64;
    let mean = reference.iter().sum::<f64>() / n;
    let var = reference.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    (sd > 1e-12).then(|| (x - mean) / sd)
}

/// Change of each point versus the latest point that is between `lag` and `lag + tol` seconds older.
/// `series` must be ascending. Points without such a predecessor are skipped.
pub fn lagged_changes(
    series: &[(i64, f64)],
    lag: i64,
    tol: i64,
    relative: bool,
) -> Vec<(i64, f64)> {
    series
        .iter()
        .filter_map(|&(ts, value)| {
            let idx = series.partition_point(|p| p.0 <= ts - lag).checked_sub(1)?;
            let (old_ts, old) = series[idx];
            if old_ts < ts - lag - tol {
                return None;
            }
            Some((
                ts,
                if relative {
                    pct(value, old)?
                } else {
                    value - old
                },
            ))
        })
        .collect()
}

/// z-score of every point against the points in the `window` seconds before it.
pub fn rolling_z(series: &[(i64, f64)], window: i64, min_n: usize) -> Vec<(i64, f64)> {
    let mut start = 0;
    let mut reference = Vec::new();
    series
        .iter()
        .enumerate()
        .filter_map(|(i, &(ts, value))| {
            while series[start].0 < ts - window {
                start += 1;
            }
            reference.clear();
            reference.extend(series[start..i].iter().map(|p| p.1));
            Some((ts, zscore(value, &reference, min_n)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zscore_uses_sample_deviation_and_rejects_thin_or_flat_references() {
        // mean 2, sample sd 1
        assert_eq!(zscore(4.0, &[1.0, 2.0, 3.0], 3), Some(2.0));
        assert_eq!(zscore(4.0, &[1.0, 2.0, 3.0], 4), None);
        assert_eq!(zscore(4.0, &[2.0, 2.0, 2.0], 3), None);
    }

    #[test]
    fn lagged_changes_skip_points_whose_reference_is_too_old() {
        // A weekend-style gap between t=20 and t=100.
        let series = [(0, 100.0), (10, 110.0), (20, 121.0), (100, 133.1)];
        // lag 10, no tolerance: t=100 has no point in [90, 90].
        assert_eq!(
            lagged_changes(&series, 10, 0, false),
            [(10, 10.0), (20, 11.0)]
        );
        // With tolerance the last print before the gap is used.
        let with_gap = lagged_changes(&series, 10, 80, true);
        assert_eq!(with_gap.len(), 3);
        assert!((with_gap[2].1 - 10.0).abs() < 1e-9, "{with_gap:?}");
    }

    #[test]
    fn rolling_z_only_looks_backwards_within_the_window() {
        let series = [(0, 1.0), (1, 2.0), (2, 3.0), (3, 10.0), (10, 0.0)];
        let z = rolling_z(&series, 3, 3);
        // t=3 sees {1,2,3}: (10-2)/1. t=10 only has t>=7 in range, which is too thin.
        assert_eq!(z, [(3, 8.0)]);
    }
}
