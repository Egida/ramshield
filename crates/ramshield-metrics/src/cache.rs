use super::*;

impl Metrics {
    /// Cached /metrics text (1s TTL): counters move on a second grain, so
    /// re-rendering 18 stanzas per scrape is pure waste. Staleness ≤ 1s is
    /// the documented Prometheus compromise (same trade as get_system_usage).
    pub fn render_prometheus_cached(&self) -> Arc<str> {
        let mut cache = self
            .metrics_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, text)) = &*cache
            && at.elapsed() < std::time::Duration::from_secs(1)
        {
            return Arc::clone(text);
        }
        let text: Arc<str> = self.render_prometheus().into();
        *cache = Some((std::time::Instant::now(), Arc::clone(&text)));
        text
    }

    /// Pre-serialized dashboard JSON, invalidated by write-path sequence
    /// bumps. A 2s poll with no new blocks costs one Arc clone instead of
    /// ~3k String clones + a serde pass. Worst case a poll is one record stale.
    pub fn get_block_log_json(&self) -> Arc<str> {
        self.seq_cached_json(&self.block_seq, &self.blocks_json_cache, || {
            serde_json::to_string(&self.get_block_log()).unwrap_or_default()
        })
    }

    pub fn get_batch_history_json(&self) -> Arc<str> {
        self.seq_cached_json(&self.batch_seq, &self.batches_json_cache, || {
            serde_json::to_string(&self.get_batch_history()).unwrap_or_default()
        })
    }

    pub(crate) fn seq_cached_json(
        &self,
        seq: &AtomicU64,
        cache: &Mutex<Option<(u64, Arc<str>)>>,
        render: impl Fn() -> String,
    ) -> Arc<str> {
        // Invariant: a cached pair is stamped with a seq read BEFORE its
        // render. Stamping after would let a write racing mid-render hide
        // behind stale text forever. A raced render simply goes unpublished
        // (seq moved -> skip store); a later reader re-renders. Overserving
        // (text newer than stamp) only costs one extra render, never staleness.
        let want = seq.load(Ordering::Acquire);
        {
            let guard = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((s, text)) = &*guard
                && *s == want
            {
                return Arc::clone(text);
            }
        }
        let text: Arc<str> = render().into();
        if seq.load(Ordering::Acquire) == want {
            *cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((want, Arc::clone(&text)));
        }
        text
    }
}
