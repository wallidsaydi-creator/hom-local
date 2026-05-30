use std::collections::VecDeque;

use hom_shared::{ERR_QUEUE_FULL, RpcError};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    Save,
    Recall,
    Cognition,
    Io,
    Main,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Hi,
    Md,
    Lo,
}

#[derive(Clone, Copy, Debug)]
pub struct QueueCaps {
    pub hi: usize,
    pub md: usize,
    pub lo: usize,
}

impl Default for QueueCaps {
    fn default() -> Self {
        Self {
            hi: 256,
            md: 1024,
            lo: 4096,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QueueSnapshot {
    pub hi: usize,
    pub md: usize,
    pub lo: usize,
    pub total: usize,
}

pub struct WeightedQueue<T> {
    hi: VecDeque<T>,
    md: VecDeque<T>,
    lo: VecDeque<T>,
    caps: QueueCaps,
    cursor: usize,
}

impl<T> WeightedQueue<T> {
    const SCHEDULE: [Priority; 21] = [
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Md,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Md,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Md,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Hi,
        Priority::Md,
        Priority::Lo,
    ];

    pub fn new(caps: QueueCaps) -> Self {
        Self {
            hi: VecDeque::new(),
            md: VecDeque::new(),
            lo: VecDeque::new(),
            caps,
            cursor: 0,
        }
    }

    pub fn push(&mut self, priority: Priority, item: T) -> Result<(), QueueFull> {
        if self.is_full(priority) {
            return Err(QueueFull {
                priority,
                len: self.len_for(priority),
                cap: self.cap_for(priority),
            });
        }
        self.queue_for_mut(priority).push_back(item);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.total_len() == 0 {
            return None;
        }

        for offset in 0..Self::SCHEDULE.len() {
            let idx = (self.cursor + offset) % Self::SCHEDULE.len();
            let priority = Self::SCHEDULE[idx];
            if self.len_for(priority) == 0 {
                continue;
            }
            self.cursor = (idx + 1) % Self::SCHEDULE.len();
            return self.queue_for_mut(priority).pop_front();
        }

        None
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            hi: self.hi.len(),
            md: self.md.len(),
            lo: self.lo.len(),
            total: self.total_len(),
        }
    }

    pub fn total_len(&self) -> usize {
        self.hi.len() + self.md.len() + self.lo.len()
    }

    pub fn is_full(&self, priority: Priority) -> bool {
        self.len_for(priority) >= self.cap_for(priority)
    }

    pub fn estimate_wait_ms(&self, priority: Priority, service_rate_per_s: f64) -> u64 {
        let rate = service_rate_per_s.max(1.0);
        ((self.len_for(priority) as f64 / rate) * 1000.0).ceil() as u64
    }

    fn len_for(&self, priority: Priority) -> usize {
        match priority {
            Priority::Hi => self.hi.len(),
            Priority::Md => self.md.len(),
            Priority::Lo => self.lo.len(),
        }
    }

    fn cap_for(&self, priority: Priority) -> usize {
        match priority {
            Priority::Hi => self.caps.hi,
            Priority::Md => self.caps.md,
            Priority::Lo => self.caps.lo,
        }
    }

    fn queue_for_mut(&mut self, priority: Priority) -> &mut VecDeque<T> {
        match priority {
            Priority::Hi => &mut self.hi,
            Priority::Md => &mut self.md,
            Priority::Lo => &mut self.lo,
        }
    }
}

#[derive(Debug)]
pub struct QueueFull {
    pub priority: Priority,
    pub len: usize,
    pub cap: usize,
}

impl QueueFull {
    pub fn into_rpc(self, retry_after_ms: u64) -> RpcError {
        RpcError {
            code: ERR_QUEUE_FULL,
            message: "queue_full".to_string(),
            data: Some(json!({
                "priority": self.priority,
                "len": self.len,
                "cap": self.cap,
                "retry_after_ms": retry_after_ms
            })),
        }
    }
}

pub fn worker_for_method(method: &str) -> Option<WorkerKind> {
    match method {
        "memory.save" => Some(WorkerKind::Save),
        "memory.embedding.upsert" => Some(WorkerKind::Save),
        "memory.recall" | "memory.recall.smart" | "memory.answer" | "memory.open" => {
            Some(WorkerKind::Recall)
        }
        "events.list"
        | "events.security"
        | "runtime.status"
        | "ledger.verify"
        | "ledger.repair_segmented"
        | "ledger.record_mutation"
        | "ledger.reconcile_mismatches"
        | "monitoring.snapshot"
        | "diagnostics.trust"
        | "diagnostics.recall"
        | "diagnostics.recall_drift"
        | "diagnostics.capability"
        | "paper.registry"
        | "exposure.policy"
        | "security.saber_dry_run"
        | "security.canary_timeline"
        | "route.certificate.issue"
        | "route.certificate.open"
        | "benchmarks.vector_recall" => Some(WorkerKind::Main),
        method
            if method.starts_with("session.")
                | method.starts_with("projects.")
                | method.starts_with("sessions.")
                | method.starts_with("nightly.")
                | method.starts_with("reasoning.")
                | method.starts_with("settings.")
                | method.starts_with("permissions.")
                | method.starts_with("providers.")
                | method.starts_with("ui.")
                | method.starts_with("gates.")
                | method.starts_with("brain.")
                | method.starts_with("benchmarks.")
                | method.starts_with("policy.")
                | method.starts_with("tool.") =>
        {
            Some(WorkerKind::Cognition)
        }
        method if method.starts_with("import.") || method.starts_with("export.") => {
            Some(WorkerKind::Io)
        }
        _ => None,
    }
}

pub fn priority_for_method(method: &str, params: &Value) -> Priority {
    match params.get("priority").and_then(Value::as_str) {
        Some("hi" | "high" | "interactive") => return Priority::Hi,
        Some("md" | "medium" | "opportunistic") => return Priority::Md,
        Some("lo" | "low" | "background") => return Priority::Lo,
        _ => {}
    }

    match method {
        "memory.save"
        | "memory.embedding.upsert"
        | "memory.recall"
        | "memory.recall.smart"
        | "memory.answer" => Priority::Hi,
        "gates.tasks.preflight" | "gates.runtime.verify" => Priority::Hi,
        "diagnostics.trust"
        | "diagnostics.recall"
        | "diagnostics.recall_drift"
        | "diagnostics.capability"
        | "paper.registry"
        | "exposure.policy"
        | "runtime.status"
        | "ledger.verify"
        | "ledger.repair_segmented"
        | "ledger.record_mutation"
        | "ledger.reconcile_mismatches"
        | "monitoring.snapshot"
        | "route.certificate.issue"
        | "route.certificate.open"
        | "benchmarks.vector_exact"
        | "benchmarks.vector_pq_candidates"
        | "benchmarks.vector_pq_exact_rerank"
        | "benchmarks.vector_recall"
        | "benchmarks.vector_thresholds"
        | "benchmarks.vector_regression_profile"
        | "benchmarks.vector_regression_compare"
        | "benchmarks.vector_regression_response"
        | "policy.vector_recall"
        | "security.saber_dry_run"
        | "security.canary_timeline"
        | "events.security"
        | "gates.plan.current"
        | "gates.runtime.snapshot"
        | "gates.decisions.list"
        | "gates.evidence.list"
        | "gates.argument.inspect" => Priority::Md,
        method if method.starts_with("session.compact") => Priority::Md,
        method if method.starts_with("reasoning.") => Priority::Md,
        method if method.starts_with("nightly.") => Priority::Lo,
        _ => Priority::Md,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct Job(Priority, usize);

    #[test]
    fn weighted_split_is_16_4_1_under_saturation() {
        let mut q = WeightedQueue::new(QueueCaps {
            hi: 64,
            md: 64,
            lo: 64,
        });
        for i in 0..32 {
            q.push(Priority::Hi, Job(Priority::Hi, i)).unwrap();
            q.push(Priority::Md, Job(Priority::Md, i)).unwrap();
            q.push(Priority::Lo, Job(Priority::Lo, i)).unwrap();
        }

        let mut counts = (0, 0, 0);
        for _ in 0..21 {
            match q.pop().unwrap().0 {
                Priority::Hi => counts.0 += 1,
                Priority::Md => counts.1 += 1,
                Priority::Lo => counts.2 += 1,
            }
        }
        assert_eq!(counts, (16, 4, 1));
    }

    #[test]
    fn preserves_fifo_within_class() {
        let mut q = WeightedQueue::new(QueueCaps::default());
        q.push(Priority::Md, Job(Priority::Md, 1)).unwrap();
        q.push(Priority::Md, Job(Priority::Md, 2)).unwrap();
        q.push(Priority::Md, Job(Priority::Md, 3)).unwrap();

        assert_eq!(q.pop().unwrap().1, 1);
        assert_eq!(q.pop().unwrap().1, 2);
        assert_eq!(q.pop().unwrap().1, 3);
    }

    #[test]
    fn does_not_starve_low_priority() {
        let mut q = WeightedQueue::new(QueueCaps::default());
        for i in 0..64 {
            q.push(Priority::Hi, Job(Priority::Hi, i)).unwrap();
            q.push(Priority::Lo, Job(Priority::Lo, i)).unwrap();
        }

        let mut saw_low = false;
        for _ in 0..21 {
            saw_low |= q.pop().unwrap().0 == Priority::Lo;
        }
        assert!(saw_low);
    }

    #[test]
    fn rejects_when_class_capacity_is_full() {
        let mut q = WeightedQueue::new(QueueCaps {
            hi: 1,
            md: 1,
            lo: 1,
        });
        q.push(Priority::Hi, Job(Priority::Hi, 1)).unwrap();
        let error = q.push(Priority::Hi, Job(Priority::Hi, 2)).unwrap_err();
        assert_eq!(error.len, 1);
        assert_eq!(error.cap, 1);
    }
}
