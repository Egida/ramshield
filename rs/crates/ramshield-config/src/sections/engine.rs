use crate::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub worker_threads: usize,
    pub ram_limit_mb: usize,
    pub shard_count: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            worker_threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            ram_limit_mb: 512,
            shard_count: 256,
        }
    }
}
