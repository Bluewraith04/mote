#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Supported collection plans.
pub enum PlanType {
    /// Allocation-only with no collection passes.
    NoGC,
    /// The hand-rolled collector (`GCController`): a full-heap mark/sweep.
    MarkSweep,
}

/// Global GC configuration surface exposed at VM startup.
#[derive(Debug, Clone)]
pub struct GCConfig {
    pub plan_type: PlanType,
    /// Live bytes past which a task faults with out of memory; `usize::MAX` means no limit.
    pub max_heap_size: usize,
    /// Least bytes allocated between collections.
    pub collection_threshold: usize,
}

impl Default for GCConfig {
    fn default() -> Self {
        Self {
            plan_type: PlanType::MarkSweep,
            max_heap_size: usize::MAX,
            collection_threshold: 4 * 1024 * 1024,
        }
    }
}

impl GCConfig {
    pub fn nogc() -> Self {
        Self {
            plan_type: PlanType::NoGC,
            ..Default::default()
        }
    }

    pub fn mark_sweep() -> Self {
        Self {
            plan_type: PlanType::MarkSweep,
            ..Default::default()
        }
    }

    pub fn with_threshold(mut self, bytes: usize) -> Self {
        self.collection_threshold = bytes;
        self
    }

    pub fn with_max_heap(mut self, bytes: usize) -> Self {
        self.max_heap_size = bytes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_config_builder() {
        let cfg = GCConfig::mark_sweep().with_threshold(1024 * 1024);
        assert_eq!(cfg.plan_type, PlanType::MarkSweep);
        assert_eq!(cfg.collection_threshold, 1024 * 1024);

        let nogc = GCConfig::nogc();
        assert_eq!(nogc.plan_type, PlanType::NoGC);
    }
}