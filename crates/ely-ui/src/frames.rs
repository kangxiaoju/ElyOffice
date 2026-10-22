//! Benchmarks only: frame times from gpui's journal (`ELY_FRAMES`).

use gpui::App;

/// Logs each busy stretch's frame times; idle logs nothing.
#[cfg(feature = "frames")]
pub(crate) fn watch(cx: &mut App) {
    use gpui::profiler::journal::{ForegroundJournalEntry, IntervalBoundary};
    use std::time::Duration;
    const EVERY: Duration = Duration::from_secs(2);
    let mut collector = cx.foreground_journal().collector();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        loop {
            cx.background_executor().timer(EVERY).await;
            let drained = collector.collect_unseen();
            let mut draws = Vec::new();
            let mut lags = Vec::new();
            for entry in drained.entries {
                if let ForegroundJournalEntry::Boundary(IntervalBoundary::Presented(shown)) = entry {
                    draws.push(shown.frame.draw_duration());
                    lags.extend(shown.dirty_to_present_duration());
                }
            }
            if !draws.is_empty() {
                log::info!("{}", summary(&mut draws, &mut lags, drained.lost));
            }
        }
    })
    .detach();
}

#[cfg(not(feature = "frames"))]
pub(crate) fn watch(_: &mut App) {
    log::warn!("ui: ELY_FRAMES needs a build with the frames feature");
}

/// One line: counts, draw and dirty-to-present percentiles in ms.
#[cfg(any(feature = "frames", test))]
fn summary(draws: &mut [std::time::Duration], lags: &mut [std::time::Duration], lost: u64) -> String {
    draws.sort();
    lags.sort();
    let ms = |all: &[std::time::Duration], p: usize| match all.len() {
        0 => f64::NAN,
        n => all[(n - 1) * p / 100].as_secs_f64() * 1000.0,
    };
    // 120 fps leaves 8.3 ms a frame.
    let late = lags.iter().filter(|lag| lag.as_secs_f64() > 1.0 / 120.0).count();
    format!(
        "ui: frames {} (lost {lost}): draw p50 {:.1} p95 {:.1} max {:.1} ms; \
         dirty to shown p50 {:.1} p95 {:.1} max {:.1} ms; over 8.3 ms {late}",
        draws.len(),
        ms(draws, 50),
        ms(draws, 95),
        ms(draws, 100),
        ms(lags, 50),
        ms(lags, 95),
        ms(lags, 100),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn the_summary_reads_percentiles_and_late_frames() {
        let mut draws: Vec<_> = (1..=100).map(Duration::from_millis).collect();
        let mut lags = vec![Duration::from_millis(4), Duration::from_millis(9), Duration::from_millis(20)];
        let line = super::summary(&mut draws, &mut lags, 0);
        assert!(line.contains("frames 100"), "{line}");
        assert!(line.contains("draw p50 50.0 p95 95.0 max 100.0"), "{line}");
        assert!(line.contains("dirty to shown p50 9.0 p95 9.0 max 20.0"), "{line}");
        assert!(line.ends_with("over 8.3 ms 2"), "{line}");
    }
}
