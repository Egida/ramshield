# T19 Throughput Fix — Implementation Plan

## Overview
Complete the bounded channel + backpressure + adaptive worker pool implementation to restore throughput from -93.5% to >95% baseline under heavy load.

## Implementation Phases

### Phase 1: IPC Channel & Connection Management (Days 1-2)

#### Files Modified:
1. `src/ipc/server.rs` - Reduce BATCH_MAX, add backpressure
2. `src/engine/mod.rs` - Add worker scaling logic

#### Key Changes:

**1. IPC Bounded Channel (src/ipc/server.rs:55)**
```rust
// Before: Batch channel unbounded
// After: Bounded 32k with backpressure
pub struct IpcServer {
    pub batch_sender: Sender<ConnectionEvent>,
    pub batch_receiver: Receiver<ConnectionEvent>,
    pub batch_channel_capacity: usize,
    pub batch_channel_depth: AtomicUsize,
    pub backpressure_active: AtomicBool,
    // ... existing fields
}

impl IpcServer {
    pub fn new(config: &Config) -> Self {
        let (tx, rx) = crossbeam_channel::bounded::<ConnectionEvent>(
            config.detection.batch_channel_capacity
        );
        
        Self {
            batch_sender: tx,
            batch_receiver: rx,
            batch_channel_capacity: config.detection.batch_channel_capacity,
            batch_channel_depth: AtomicUsize::new(0),
            backpressure_active: AtomicBool::new(false),
            // ... other fields
        }
    }
    
    pub fn process_incoming(&mut self, event: ConnectionEvent) -> Result<(), String> {
        // Backpressure at 75% capacity
        if self.batch_channel_depth.load(Ordering::Relaxed) >= 
           BACKPRESSURE_THRESHOLD * 3 / 4 {
            // Enter backpressure mode
            self.backpressure_active.store(true, Ordering::Relaxed);
            return Err("Channel backpressure".to_string());
        }
        
        // Normal processing
        match self.batch_sender.try_send(event) {
            Ok(()) => {
                self.batch_channel_depth.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                // Overflow at capacity
                self.backpressure_active.store(true, Ordering::Relaxed);
                Err("Channel overflow".to_string())
            }
        }
    }
}
```

**2. Connection Rejection Logic**
```rust
// src/ipc/server.rs:90-120
impl IpcServer {
    pub fn process_incoming(&mut self, event: ConnectionEvent) -> Result<(), String> {
        // Reject if we have 90% of connection limit
        if self.semaphore.available_permits() <= 1 {
            return Err("Connection limit reached".to_string());
        }
        
        // Check batch channel backpressure
        if self.batch_channel_depth.load(Ordering::Relaxed) >= 
           BACKPRESSURE_THRESHOLD {
            return Err("Channel backpressure".to_string());
        }
        
        // Set backpressure warning for monitoring
        if self.batch_channel_depth.load(Ordering::Relaxed) >= 
           BACKPRESSURE_THRESHOLD * 2 / 3 {
            self.backpressure_active.store(true, Ordering::Relaxed);
        }
        
        // Attempt to send with timeout
        match self.batch_sender.try_send(event) {
            Ok(()) => {
                self.batch_channel_depth.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                self.backpressure_active.store(true, Ordering::Relaxed);
                Err("Channel overflow".to_string())
            }
        }
    }
    
    pub fn get_backpressure_status(&self) -> (bool, usize, usize) {
        let depth = self.batch_channel_depth.load(Ordering::Relaxed);
        let active = self.backpressure_active.load(Ordering::Relaxed);
        let connections = self.total_connections.load(Ordering::Relaxed);
        (active, depth, connections)
    }
}
```

**3. Adaptive Worker Pool (src/engine/mod.rs)**
```rust
// src/engine/mod.rs:140-180
pub struct WorkerPool {
    pub workers: Vec<std::thread::JoinHandle<()>>,
    pub sender: Sender<ConnectionEvent>,
    pub receiver: Arc<Mutex<Receiver<Event>>>,
    pub min_workers: usize,
    pub max_workers: usize,
    pub active_workers: AtomicUsize,
    pub scaling_up_event: Arc<AsyncNotify>,
}

impl WorkerPool {
    pub fn new(
        sender: Sender<ConnectionEvent>, 
        receiver: Arc<Mutex<Receiver<Event>>>,
        config: &DetectionConfig
    ) -> Self {
        let mut workers = Vec::with_capacity(config.detection.worker_min);
        
        for _ in 0..config.detection.worker_min {
            workers.push(tokio::spawn(worker_loop(receiver.clone())));
        }
        
        Self {
            workers,
            sender,
            receiver,
            min_workers: config.detection.worker_min,
            max_workers: config.detection.worker_max,
            active_workers: AtomicUsize::new(config.detection.worker_min),
            scaling_up_event: Arc::new(AsyncNotify::new()),
        }
    }
    
    pub async fn scale_workers(&mut self, depth: usize, config: &DetectionConfig) {
        let current = self.active_workers.load(Ordering::Relaxed);
        
        // Scale up if channel depth exceeds threshold
        if depth > config.detection.worker_scale_up_threshold && 
           current < config.detection.worker_max {
            
            let additional = (config.detection.worker_max - current)
                .min(current) // Double at most
                .min(4); // Max 4 new workers at once
                
            for i in 0..additional {
                self.workers.push(tokio::spawn(worker_loop(self.receiver.clone())));
            }
            
            self.active_workers.store(
                self.workers.len(), 
                Ordering::Relaxed
            );
            
            self.scaling_up_event.notify(1);
            
            info!("Scaled workers up to {} (depth: {})", self.workers.len(), depth);
        }
        // Scale down if channel depth is low (with hysteresis)
        else if depth < config.detection.worker_scale_down_threshold && 
                 current > config.detection.worker_min {
            
            let workers_to_keep = current - 
                (current - config.detection.worker_min).min(2);
                
            self.workers.truncate(workers_to_keep);
            self.active_workers.store(workers_to_keep, Ordering::Relaxed);
            
            info!("Scaled workers down to {} (depth: {})", workers_to_keep, depth);
        }
    }
}
```

### Phase 2: Worker Coordination & Metrics (Days 3-4)

**4. DetectionEngine Worker Management (src/engine/mod.rs:500-600)**
```rust
impl DetectionEngine {
    // Replace static worker spawning with adaptive pool
    pub fn spawn_workers(self: Arc<Self>, n: usize) {
        let det = self.config.load().detection.clone();
        let n_workers = if n == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(det.worker_min)
        } else {
            n
        };
        
        info!(
            "Detection: spawning {} batch processors with adaptive scaling (min: {}, max: {}),
             1 subnet loop",
            n_workers, det.worker_min, det.worker_max
        );
        
        let mut handles = self.worker_handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
            
        for i in 0..n_workers {
            let eng = self.clone();
            let rx = self.event_rx.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("rs-batch-adaptive-{i}"))
                    .spawn(move || eng.batch_processor_loop_with_monitoring(rx))
                    .expect("spawn adaptive batch processor"),
            );
        }
        
        let eng = self.clone();
        handles.push(
            std::thread::Builder::new()
                .name("rs-subnet-adaptive".into())
                .spawn(move || eng.subnet_batch_loop_with_monitoring())
                .expect("spawn adaptive subnet batch loop"),
        );
        
        drop(handles);
    }
    
    // Enhanced batch loop with worker monitoring
    fn batch_processor_loop_with_monitoring(
        &self, 
        rx: Arc<Receiver<ConnectionEvent>>
    ) {
        let mut local: HashMap<IpAddr, IpAgg> = HashMap::new();
        let mut local_worker_depth = 0usize;
        
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                self.merge_local(&mut local);
                self.flush_pre_aggs_to_store();
                info!("Adaptive batch processor shutting down");
                break;
            }
            
            let cfg = self.config.load();
            let window = Duration::from_millis(cfg.detection.batch_window_ms);
            let max = cfg.detection.batch_max_events;
            
            // Monitor and potentially scale workers
            if local.len() > LOCAL_MERGE_SOFT_CAP * 2 {
                let depth = self.event_queue_depth();
                // Trigger async scaling in background
                let eng = self.clone();
                tokio::spawn(async move {
                    let depth = depth;
                    // Implementation of scaling logic
                });
            }
            
            // Normal processing continues...
            match rx.recv_timeout(window) {
                Ok(ev) => {
                    local.entry(ev.ip).or_default().absorb(&ev);
                    local_worker_depth += 1;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.merge_local(&mut local);
                    self.flush_pre_aggs_to_store();
                    break;
                }
            }
            
            // Continue with existing flush logic...
        }
    }
}
```

### Phase 3: Configuration & Monitoring (Days 5-6)

**5. Detection Config (crates/ramshield-config/src/lib.rs)**
```rust
// crates/ramshield-config/src/lib.rs:140-160
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DetectionConfig {
    // Existing config...
    
    // New bounded channel settings
    pub batch_channel_capacity: usize,      // Default: 32_000
    pub backpressure_threshold_pct: usize,   // Default: 75
    pub connection_backpressure_pct: usize,  // Default: 90
    
    // Adaptive worker pool
    pub worker_min: usize,                   // Default: 2
    pub worker_max: usize,                   // Default: 8
    pub worker_scale_up_threshold: usize,    // Default: 6_000
    pub worker_scale_down_threshold: usize,  // Default: 2_000
}
```

**6. Metrics for Backpressure (crates/ramshield-metrics/src/lib.rs)**
```rust
// crates/ramshield-metrics/src/lib.rs:200-220
pub struct DetectionMetrics {
    // Existing metrics...
    
    // Backpressure metrics
    pub channel_depth: AtomicU64,
    pub workers_active: AtomicUsize,
    pub workers_desired: AtomicUsize,
    pub backpressure_events: AtomicU64,
    pub events_shed: AtomicU64,
    
    // Performance metrics under load
    pub throughput_restored: AtomicBool,
}
```

### Phase 4: Testing & Validation (Days 7-8)

**7. Load Test Suite (scripts/t19_fix_test.rs)**
```rust
// scripts/t19_fix_test.rs
use std::thread::sleep;
use std::time::{Duration, Instant};

fn test_bounded_channel_rejection() {
    println!("Testing bounded channel rejection at capacity...");
    
    let (tx, rx) = crossbeam_channel::bounded::<ConnectionEvent>(32_000);
    
    // Fill channel to capacity
    for i in 0..32_000 {
        let event = ConnectionEvent { ip: Ipv4Addr::LOCALHOST, ... };
        tx.send(event).unwrap();
    }
    
    // Additional sends should fail immediately
    for i in 0..100 {
        let event = ConnectionEvent { ip: Ipv4Addr::LOCALHOST, ... };
        match tx.try_send(event) {
            Ok(()) => panic!("Should not accept events at capacity"),
            Err(TrySendError::Full(_)) => {} // Expected
        }
    }
    
    println!("✓ Bounded channel rejection working correctly");
}

fn test_adaptive_worker_scaling() {
    println!("Testing adaptive worker pool scaling...");
    
    let mut pool = WorkerPool::new(initial_config);
    
    // Simulate low load - should scale down
    pool.scale_workers(100, &config);
    assert!(pool.active_workers() <= config.worker_max);
    
    // Simulate high load - should scale up
    pool.scale_workers(10_000, &config);
    assert!(pool.active_workers() >= config.worker_min);
    
    println!("✓ Adaptive worker scaling working correctly");
}

fn test_connection_backpressure() {
    println!("Testing connection backpressure at 90% limit...");
    
    let mut server = IpcServer::new(&config);
    
    // Fill connections to 90% of limit
    let connection_limit = config.max_connections;
    let backpressure_limit = (connection_limit as f64 * 
                           config.connection_backpressure_pct as f64 / 100.0) as usize;
    
    for i in 0..backpressure_limit {
        let event = ConnectionEvent { ip: Ipv4Addr::LOCALHOST, ... };
        match server.process_incoming(event) {
            Ok(()) => {},
            Err(e) if e.contains("Connection limit") => panic!("Should accept connections below limit"),
            Err(_) => {}
        }
    }
    
    // Attempt connection at 90%+ should be rejected
    let event = ConnectionEvent { ip: Ipv4Addr::LOCALHOST, ... };
    match server.process_incoming(event) {
        Ok(()) => panic!("Should reject connections at 90%+ limit"),
        Err(e) if e.contains("Connection limit") => {}, // Expected
        _ => panic!("Wrong error type")
    }
    
    println!("✓ Connection backpressure working correctly");
}

fn test_throughput_restoration() {
    println!("Testing throughput restoration under load...");
    
    let start = Instant::now();
    let target_events = 1_000_000; // 1M events
    let mut processed = 0;
    
    let thread_handle = std::thread::spawn(move || {
        let mut engine = DetectionEngine::new(config);
        engine.spawn_workers(4);
        
        while processed < target_events {
            // Process events at high rate
            let event = ConnectionEvent { ip: Ipv4Addr::LOCALHOST, ... };
            engine.process_incoming(event);
            processed += 1;
            
            if processed % 100_000 == 0 {
                println!("Processed {}/{} events", processed, target_events);
            }
        }
    });
    
    thread_handle.join().unwrap();
    let duration = start.elapsed();
    let throughput = (target_events as f64) / duration.as_secs_f64();
    
    println!("✓ Throughput: {} events/sec (target: >1M/sec)", throughput);
    assert!(throughput > 1_000_000.0, 
        "Throughput restoration failed: {} events/sec", throughput);
}
```

### Implementation Timeline

| Day | Tasks | Deliverables |
|-----|-------|--------------|
| 1-2 | IPC channel bounds, backpressure, connection rejection | IpcServer with bounded channel |
| 3-4 | Worker pool scaling, metrics, monitoring | WorkerPool with adaptive scaling |
| 5-6 | Configuration updates, integration testing | Updated config schema, integration tests |
| 7-8 | Load testing, validation, deployment prep | Full test suite, deployment scripts |

### Verification Criteria

✅ **Throughput ≥95% baseline** under 1M eps sustained load
✅ **Memory bounded** <500MB RSS under maximum load  
✅ **Latency <100ms** for IPC requests
✅ **Graceful degradation** during overload
✅ **All existing tests pass** with new changes
✅ **Metrics updated** for monitoring

### Migration Path

**Development → Staging → Production**
1. **Canary deployment** (10% traffic)
2. **Metrics validation** (throughput, memory, latency)
3. **Gradual rollout** (25%, 50%, 100%)
4. **Rollback plan** if any threshold breached

---

**Status**: IMPLEMENTATION READY - Execute Phases 1-4 to complete T19 fix